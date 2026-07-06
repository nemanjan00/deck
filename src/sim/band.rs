//! IQ band compositor: synthesizes a populated slice of spectrum — NBFM
//! voice babble, POCSAG/AFSK/RTTY bursts, SSB, AM, WFM, carriers, noise —
//! so the whole RX chain (tuner → DSP → real decoders) runs without an SDR.

use crate::dsp::{Nco, Rng};
use rustfft::num_complex::Complex32;

/// FM modulator (phase accumulator).
pub struct FmMod {
    phase: f32,
    k: f32,
}

impl FmMod {
    pub fn new(fs: f64, dev: f64) -> Self {
        Self {
            phase: 0.0,
            k: (2.0 * std::f64::consts::PI * dev / fs) as f32,
        }
    }

    #[inline]
    pub fn step(&mut self, x: f32) -> Complex32 {
        self.phase += self.k * x;
        if self.phase > std::f32::consts::PI {
            self.phase -= 2.0 * std::f32::consts::PI;
        } else if self.phase < -std::f32::consts::PI {
            self.phase += 2.0 * std::f32::consts::PI;
        }
        Complex32::new(self.phase.cos(), self.phase.sin())
    }
}

/// Voice-ish audio: gliding formants under a syllabic envelope.
pub struct Babble {
    rng: Rng,
    f: [f32; 3],
    ftgt: [f32; 3],
    ph: [f32; 3],
    env: f32,
    env_tgt: f32,
    hold: u32,
    fs: f32,
}

impl Babble {
    pub fn new(fs: f64, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut f = [0.0f32; 3];
        for (i, v) in f.iter_mut().enumerate() {
            *v = 250.0 * (i as f32 + 1.2) + rng.range_f64(0.0, 200.0) as f32;
        }
        Self {
            rng,
            f,
            ftgt: f,
            ph: [0.0; 3],
            env: 0.0,
            env_tgt: 1.0,
            hold: 0,
            fs: fs as f32,
        }
    }

    #[inline]
    pub fn sample(&mut self) -> f32 {
        if self.hold == 0 {
            // new "syllable" every 80–300 ms
            self.hold = self.rng.range_u32(
                (self.fs * 0.08) as u32,
                (self.fs * 0.3) as u32,
            );
            self.env_tgt = if self.rng.f64() < 0.75 { 1.0 } else { 0.05 };
            for i in 0..3 {
                self.ftgt[i] =
                    (300.0 + 700.0 * i as f32) * self.rng.range_f64(0.7, 1.4) as f32;
            }
        }
        self.hold -= 1;
        self.env += (self.env_tgt - self.env) * (30.0 / self.fs);
        let mut s = 0.0f32;
        for i in 0..3 {
            self.f[i] += (self.ftgt[i] - self.f[i]) * (8.0 / self.fs);
            self.ph[i] += 2.0 * std::f32::consts::PI * self.f[i] / self.fs;
            if self.ph[i] > std::f32::consts::PI {
                self.ph[i] -= 2.0 * std::f32::consts::PI;
            }
            s += self.ph[i].sin() * [0.6, 0.3, 0.15][i];
        }
        s * self.env
    }
}

/// On/off burst schedule with click-free ramping.
pub struct Schedule {
    period: f64,
    duty: f64,
    t: f64,
    amp: f32,
}

impl Schedule {
    pub fn new(period: f64, duty: f64, phase: f64) -> Self {
        Self {
            period,
            duty,
            t: phase * period,
            amp: 0.0,
        }
    }

    pub fn always() -> Self {
        Self::new(1.0, 2.0, 0.0)
    }

    /// Deterministic per-channel pattern (scanner demo).
    pub fn for_channel(hz: u64, seed: u64) -> Self {
        let mut rng = Rng::new(hz ^ seed.rotate_left(17) ^ 0xDECC);
        let period = rng.range_f64(4.0, 11.0);
        let duty = rng.range_f64(0.15, 0.55);
        Self::new(period, duty, rng.f64())
    }

    #[inline]
    fn gain(&mut self, dt: f64) -> f32 {
        self.t += dt;
        let on = (self.t % self.period) < self.period * self.duty;
        let tgt = if on { 1.0 } else { 0.0 };
        self.amp += (tgt - self.amp) * 0.002;
        self.amp
    }
}

/// Repeating bitstream with a per-burst gap, at a fixed baud rate.
pub struct BitLoop {
    bits: Vec<bool>,
    idx: usize,
    sps: f64,
    acc: f64,
    gap: f64,
    gap_left: f64,
    regenerate: Box<dyn FnMut() -> Vec<bool> + Send>,
}

impl BitLoop {
    pub fn new(
        fs: f64,
        baud: f64,
        gap_s: f64,
        mut regenerate: Box<dyn FnMut() -> Vec<bool> + Send>,
    ) -> Self {
        let bits = regenerate();
        Self {
            bits,
            idx: 0,
            sps: fs / baud,
            acc: 0.0,
            gap: gap_s,
            gap_left: 0.0,
            regenerate,
        }
    }

    /// (level −1/0/+1, active) advanced by one sample; 0 during the gap.
    #[inline]
    fn step(&mut self, fs: f64) -> (f32, bool) {
        if self.gap_left > 0.0 {
            self.gap_left -= 1.0 / fs;
            if self.gap_left <= 0.0 {
                self.bits = (self.regenerate)();
                self.idx = 0;
                self.acc = 0.0;
            }
            return (0.0, false);
        }
        self.acc += 1.0;
        if self.acc >= self.sps {
            self.acc -= self.sps;
            self.idx += 1;
            if self.idx >= self.bits.len() {
                self.gap_left = self.gap;
                return (0.0, false);
            }
        }
        // POCSAG convention: 0 bit = +deviation
        (if self.bits[self.idx] { -1.0 } else { 1.0 }, true)
    }
}

/// One signal in the band.
pub enum Component {
    Noise {
        rng: Rng,
        amp: f32,
    },
    Carrier {
        nco: Nco,
        amp: f32,
        drift_hz_s: f64,
        freq: f64,
        n: u64,
    },
    NbfmVoice {
        nco: Nco,
        fm: FmMod,
        voice: Babble,
        sched: Schedule,
        amp: f32,
    },
    /// FSK/NRZ bits as FM (POCSAG, RTTY): shaped square into the modulator.
    FmBits {
        nco: Nco,
        fm: FmMod,
        bits: BitLoop,
        lpf: f32,
        lpf_state: f32,
        amp: f32,
    },
    /// AFSK1200 tones inside NBFM (APRS).
    AfskFm {
        nco: Nco,
        fm: FmMod,
        levels: BitLoop,
        tone_ph: f32,
        amp: f32,
    },
    Am {
        nco: Nco,
        voice: Babble,
        amp: f32,
    },
    Wfm {
        nco: Nco,
        fm: FmMod,
        voice: Babble,
        pilot_ph: f32,
        amp: f32,
    },
    /// SSB as gliding analytic tones (correct sideband placement).
    SsbVoice {
        nco: Nco,
        voice: Babble,
        tone: Nco,
        sched: Schedule,
        usb: bool,
        amp: f32,
    },
}

impl Component {
    pub fn add(&mut self, out: &mut [Complex32], fs: f64) {
        let dt = 1.0 / fs;
        match self {
            Component::Noise { rng, amp } => {
                for o in out.iter_mut() {
                    *o += Complex32::new(
                        rng.gauss() as f32 * *amp,
                        rng.gauss() as f32 * *amp,
                    );
                }
            }
            Component::Carrier {
                nco,
                amp,
                drift_hz_s,
                freq,
                n,
            } => {
                for o in out.iter_mut() {
                    *o += nco.next() * *amp;
                    *n += 1;
                }
                if *drift_hz_s != 0.0 {
                    *freq += *drift_hz_s * out.len() as f64 * dt;
                    nco.set_freq(*freq, fs);
                }
            }
            Component::NbfmVoice {
                nco,
                fm,
                voice,
                sched,
                amp,
            } => {
                for o in out.iter_mut() {
                    let g = sched.gain(dt);
                    if g > 0.001 {
                        let a = voice.sample();
                        *o += fm.step(a) * nco.next() * (*amp * g);
                    } else {
                        let _ = nco.next();
                    }
                }
            }
            Component::FmBits {
                nco,
                fm,
                bits,
                lpf,
                lpf_state,
                amp,
            } => {
                for o in out.iter_mut() {
                    let (lvl, active) = bits.step(fs);
                    *lpf_state += (*lpf) * (lvl - *lpf_state);
                    if active {
                        *o += fm.step(*lpf_state) * nco.next() * *amp;
                    } else {
                        let _ = nco.next();
                    }
                }
            }
            Component::AfskFm {
                nco,
                fm,
                levels,
                tone_ph,
                amp,
            } => {
                for o in out.iter_mut() {
                    let (lvl, active) = levels.step(fs);
                    if active {
                        // mark 1200 Hz, space 2200 Hz, continuous phase
                        let f = if lvl > 0.0 { 1200.0 } else { 2200.0 };
                        *tone_ph += (2.0 * std::f64::consts::PI * f / fs) as f32;
                        if *tone_ph > std::f32::consts::PI {
                            *tone_ph -= 2.0 * std::f32::consts::PI;
                        }
                        let a = tone_ph.sin() * 0.9;
                        *o += fm.step(a) * nco.next() * *amp;
                    } else {
                        let _ = nco.next();
                    }
                }
            }
            Component::Am { nco, voice, amp } => {
                for o in out.iter_mut() {
                    let m = 1.0 + 0.7 * voice.sample();
                    *o += nco.next() * (m * *amp * 0.5);
                }
            }
            Component::Wfm {
                nco,
                fm,
                voice,
                pilot_ph,
                amp,
            } => {
                for o in out.iter_mut() {
                    *pilot_ph += (2.0 * std::f64::consts::PI * 1000.0 / fs) as f32;
                    if *pilot_ph > std::f32::consts::PI {
                        *pilot_ph -= 2.0 * std::f32::consts::PI;
                    }
                    let a = 0.6 * voice.sample() + 0.25 * pilot_ph.sin();
                    *o += fm.step(a) * nco.next() * *amp;
                }
            }
            Component::SsbVoice {
                nco,
                voice,
                tone,
                sched,
                usb,
                amp,
            } => {
                for o in out.iter_mut() {
                    let g = sched.gain(dt);
                    let a = voice.sample().abs() + 0.15;
                    let t = tone.next();
                    let t = if *usb { t } else { t.conj() };
                    *o += t * nco.next() * (a * *amp * g);
                }
            }
        }
    }
}

pub struct BandProfile {
    pub components: Vec<Component>,
}

/// Build the band content for a sim profile. `off` = channel − center.
pub fn build_profile(
    profile: &str,
    fs: f64,
    off: f64,
    seed: u64,
    channel_hz: u64,
    callsign: String,
    address: u32,
    message: String,
) -> BandProfile {
    let mut c: Vec<Component> = vec![Component::Noise {
        rng: Rng::new(seed ^ 0xA5),
        amp: 0.015,
    }];
    let nbfm_voice = |off: f64, seed: u64, sched: Schedule| Component::NbfmVoice {
        nco: Nco::new(off, fs),
        fm: FmMod::new(fs, 3000.0),
        voice: Babble::new(fs, seed),
        sched,
        amp: 0.5,
    };
    let pocsag_bursts = |off: f64, addr: u32, msg: String| {
        let mut n = 0u32;
        Component::FmBits {
            nco: Nco::new(off, fs),
            fm: FmMod::new(fs, 4500.0),
            bits: BitLoop::new(
                fs,
                1200.0,
                3.5,
                Box::new(move || {
                    n = n.wrapping_add(1);
                    let text = format!("{msg} #{n}");
                    let content = if n % 4 == 3 {
                        super::pocsag::Content::Numeric("0612345678")
                    } else {
                        super::pocsag::Content::Alpha(&text)
                    };
                    super::pocsag::transmission_bits(addr + (n % 3), n as u8 % 4, content)
                }),
            ),
            lpf: 0.35,
            lpf_state: 0.0,
            amp: 0.55,
        }
    };
    match profile {
        "wfm" => c.push(Component::Wfm {
            nco: Nco::new(off, fs),
            fm: FmMod::new(fs, 65_000.0),
            voice: Babble::new(fs, seed ^ 2),
            pilot_ph: 0.0,
            amp: 0.6,
        }),
        "am" => c.push(Component::Am {
            nco: Nco::new(off, fs),
            voice: Babble::new(fs, seed ^ 3),
            amp: 0.7,
        }),
        "usb" | "lsb" => c.push(Component::SsbVoice {
            nco: Nco::new(off, fs),
            voice: Babble::new(fs, seed ^ 4),
            tone: Nco::new(900.0, fs),
            sched: Schedule::new(7.0, 0.6, 0.0),
            usb: profile == "usb",
            amp: 0.6,
        }),
        "rtty" => {
            let msg = message.clone();
            c.push(Component::FmBits {
                // RTTY = 170 Hz shift FSK; audio tones land at 1415/1585 Hz
                // in a USB receiver tuned `channel` (minimodem's defaults).
                nco: Nco::new(off + 1500.0, fs),
                fm: FmMod::new(fs, 85.0),
                bits: BitLoop::new(
                    fs,
                    45.45,
                    1.2,
                    Box::new(move || {
                        super::baudot::encode(&format!("RYRYRY DE DECK {msg} "))
                    }),
                ),
                lpf: 1.0,
                lpf_state: 0.0,
                amp: 0.6,
            });
        }
        "pocsag" => {
            c.push(pocsag_bursts(off, address, message.clone()));
            c.push(pocsag_bursts(off + 200_000.0, address + 100, "2nd channel".into()));
        }
        "aprs" => {
            let mut n = 0u32;
            let cs = callsign.clone();
            c.push(Component::AfskFm {
                nco: Nco::new(off, fs),
                fm: FmMod::new(fs, 3000.0),
                levels: BitLoop::new(
                    fs,
                    1200.0,
                    2.5,
                    Box::new(move || {
                        n = n.wrapping_add(1);
                        let info = match n % 3 {
                            0 => format!(">deck sim online #{n}"),
                            1 => "!4447.40N/02027.60E>073/036 deck mobile".to_string(),
                            _ => format!("T#{:03},123,045,678,010,090,00000000", n % 1000),
                        };
                        let f = super::ax25::frame_bytes(&cs, "APDECK", &["WIDE1-1"], &info);
                        super::ax25::nrzi(&super::ax25::hdlc_bits(&f, 24, 3))
                            .into_iter()
                            .map(|mark| mark) // NRZI levels drive the tones
                            .collect()
                    }),
                ),
                tone_ph: 0.0,
                amp: 0.55,
            });
        }
        "waterfall" => {
            c.push(Component::Carrier {
                nco: Nco::new(off, fs),
                amp: 0.2,
                drift_hz_s: 0.0,
                freq: off,
                n: 0,
            });
            c.push(Component::Carrier {
                nco: Nco::new(off + 620_000.0, fs),
                amp: 0.12,
                drift_hz_s: 2500.0,
                freq: off + 620_000.0,
                n: 0,
            });
            c.push(nbfm_voice(off - 480_000.0, seed ^ 9, Schedule::new(5.0, 0.5, 0.2)));
            c.push(pocsag_bursts(off + 300_000.0, address, message.clone()));
        }
        // nfm, scanner, voice modes, anything else: busy NBFM band
        _ => {
            let sched = if profile == "scanner" {
                Schedule::for_channel(channel_hz, seed)
            } else {
                Schedule::new(7.0, 0.6, 0.0)
            };
            c.push(nbfm_voice(off, seed ^ 1, sched));
            c.push(nbfm_voice(off - 350_000.0, seed ^ 7, Schedule::new(9.0, 0.25, 0.5)));
            c.push(Component::Carrier {
                nco: Nco::new(off + 421_000.0, fs),
                amp: 0.05,
                drift_hz_s: 0.0,
                freq: off + 421_000.0,
                n: 0,
            });
        }
    }
    BandProfile { components: c }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::SpectrumFft;

    fn band_energy_at(iq: &[Complex32], fs: f64, freq: f64) -> f32 {
        let fft = SpectrumFft::new(1024);
        let spec = fft.iq_db(&iq[iq.len() - 1024..]);
        // iq_db is fft-shifted: index 512 = DC
        let bin = (512.0 + freq / fs * 1024.0).round() as usize;
        spec[bin.saturating_sub(2).min(1023)..(bin + 3).min(1024)]
            .iter()
            .fold(f32::MIN, |a, &b| a.max(b))
    }

    #[test]
    fn nfm_profile_puts_signal_on_channel() {
        let fs = 2_400_000.0;
        let off = -600_000.0;
        let mut p = build_profile("nfm", fs, off, 1, 145_500_000, "N0DECK".into(), 1, "hi".into());
        let mut iq = vec![Complex32::default(); 65536];
        for c in &mut p.components {
            c.add(&mut iq, fs);
        }
        let on = band_energy_at(&iq, fs, off);
        let empty = band_energy_at(&iq, fs, off + 150_000.0);
        assert!(
            on > empty + 8.0,
            "channel should carry energy (on {on:.1} dB, empty {empty:.1} dB)"
        );
    }

    #[test]
    fn rtty_profile_marks_spot() {
        let fs = 2_400_000.0;
        let mut p = build_profile("rtty", fs, 10_000.0, 2, 7_646_000, "X".into(), 1, "TEST".into());
        let mut iq = vec![Complex32::default(); 65536];
        for c in &mut p.components {
            c.add(&mut iq, fs);
        }
        let on = band_energy_at(&iq, fs, 11_500.0);
        let off = band_energy_at(&iq, fs, 300_000.0);
        assert!(on > off + 8.0, "rtty carrier missing ({on:.1} vs {off:.1})");
    }
}
