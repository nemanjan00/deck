//! `deck simgen` — the built-in signal simulator.
//!
//! Three output classes:
//! * IQ band (`--mode iq-band --profile <mode>`): a populated slice of
//!   spectrum; deck's sim device feeds this through the REAL RX chain and
//!   real decoders. Also usable standalone as a general IQ generator.
//! * decoder-input audio (`--mode pocsag|aprs|rtty`): s16le mono bursts that
//!   multimon-ng / minimodem decode directly (doctor's selftest uses this).
//! * decoded lines (`--lines`): synthetic decoder output for any mode.

pub mod adsb;
pub mod ax25;
pub mod band;
pub mod baudot;
pub mod pocsag;
pub mod voice;

use crate::dsp::Rng;
use anyhow::{bail, Result};
use clap::Args;
use rustfft::num_complex::Complex32;
use std::io::Write;
use std::time::{Duration, Instant};

#[derive(Args, Debug, Clone)]
pub struct SimArgs {
    /// mode key (pocsag, aprs, rtty, dmr, …) or "iq-band"
    #[arg(long)]
    pub mode: String,
    /// emit decoded-output lines instead of signal
    #[arg(long)]
    pub lines: bool,
    /// iq-band content profile (defaults to --mode)
    #[arg(long, default_value = "")]
    pub profile: String,
    /// device center frequency in Hz (iq-band)
    #[arg(long, default_value_t = 0)]
    pub center: u64,
    /// tuned channel frequency in Hz (iq-band)
    #[arg(long, default_value_t = 0)]
    pub channel: u64,
    /// sample rate (audio modes: 22050; iq-band: 2400000)
    #[arg(long, default_value_t = 0)]
    pub rate: u32,
    /// iq output format: cu8 | cs16 | f32
    #[arg(long, default_value = "cu8")]
    pub format: String,
    #[arg(long, default_value_t = 0xDECC)]
    pub seed: u64,
    /// bursts / calls / seconds to emit (0 = endless)
    #[arg(long, default_value_t = 0)]
    pub count: u32,
    /// no real-time pacing (file generation, CI)
    #[arg(long)]
    pub fast: bool,
    #[arg(long, default_value = "N0DECK-9")]
    pub callsign: String,
    #[arg(long, default_value_t = 1234567)]
    pub address: u32,
    #[arg(long, default_value = "deck sim: all systems nominal")]
    pub message: String,
}

/// Real-time pacing for sample streams.
struct Throttle {
    start: Instant,
    sent: u64,
    per_sec: f64,
    fast: bool,
}

impl Throttle {
    fn new(per_sec: f64, fast: bool) -> Self {
        Self {
            start: Instant::now(),
            sent: 0,
            per_sec,
            fast,
        }
    }

    fn pace(&mut self, n: u64) {
        self.sent += n;
        if self.fast {
            return;
        }
        let target = self.start + Duration::from_secs_f64(self.sent as f64 / self.per_sec);
        let now = Instant::now();
        if target > now {
            std::thread::sleep(target - now);
        }
    }
}

fn out_ok(r: std::io::Result<()>) -> Result<bool> {
    match r {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn run(args: SimArgs) -> Result<()> {
    if args.lines {
        return run_lines(&args);
    }
    match args.mode.as_str() {
        "iq-band" => run_iq_band(&args),
        "pocsag" => run_audio_pocsag(&args),
        "aprs" => run_audio_aprs(&args),
        "rtty" => run_audio_rtty(&args),
        "dmr" | "ysf" | "dstar" | "nxdn" | "p25" | "m17" | "adsb" => {
            // these can't be synthesized at signal level — line feed instead
            run_lines(&args)
        }
        other => bail!("simgen: unknown --mode {other} (try --lines)"),
    }
}

// ------------------------------------------------------------------ iq-band

fn run_iq_band(args: &SimArgs) -> Result<()> {
    let rate = if args.rate == 0 { 2_400_000 } else { args.rate };
    let fs = f64::from(rate);
    let center = if args.center == 0 { 433_920_000 } else { args.center };
    let channel = if args.channel == 0 { center } else { args.channel };
    let profile = if args.profile.is_empty() {
        args.mode.clone()
    } else {
        args.profile.clone()
    };
    let off = channel as f64 - center as f64;
    let mut prof = band::build_profile(
        &profile,
        fs,
        off,
        args.seed,
        channel,
        args.callsign.clone(),
        args.address,
        args.message.clone(),
    );

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut th = Throttle::new(fs, args.fast);
    let mut buf = vec![Complex32::default(); 8192];
    let mut bytes: Vec<u8> = Vec::with_capacity(8192 * 4);
    let deadline = (args.count > 0).then(|| u64::from(args.count) * rate as u64);
    let mut total: u64 = 0;

    loop {
        buf.iter_mut().for_each(|c| *c = Complex32::default());
        for c in &mut prof.components {
            c.add(&mut buf, fs);
        }
        bytes.clear();
        match args.format.as_str() {
            "cu8" => {
                for s in &buf {
                    bytes.push((s.re.clamp(-1.0, 1.0) * 100.0 + 127.5) as u8);
                    bytes.push((s.im.clamp(-1.0, 1.0) * 100.0 + 127.5) as u8);
                }
            }
            "cs16" => {
                for s in &buf {
                    bytes.extend_from_slice(
                        &((s.re.clamp(-1.0, 1.0) * 24000.0) as i16).to_le_bytes(),
                    );
                    bytes.extend_from_slice(
                        &((s.im.clamp(-1.0, 1.0) * 24000.0) as i16).to_le_bytes(),
                    );
                }
            }
            "f32" => {
                for s in &buf {
                    bytes.extend_from_slice(&s.re.to_le_bytes());
                    bytes.extend_from_slice(&s.im.to_le_bytes());
                }
            }
            other => bail!("simgen: unknown --format {other}"),
        }
        if !out_ok(out.write_all(&bytes))? {
            return Ok(());
        }
        total += buf.len() as u64;
        th.pace(buf.len() as u64);
        if let Some(d) = deadline {
            if total >= d {
                return Ok(());
            }
        }
    }
}

// ------------------------------------------------------- decoder-input audio

fn write_audio(
    out: &mut impl Write,
    th: &mut Throttle,
    samples: &[f32],
) -> Result<bool> {
    let bytes = crate::dsp::f32_to_s16(samples);
    if !out_ok(out.write_all(&bytes))? {
        return Ok(false);
    }
    th.pace(samples.len() as u64);
    Ok(true)
}

fn gap(rng: &mut Rng, rate: u32, seconds: f64) -> Vec<f32> {
    (0..(rate as f64 * seconds) as usize)
        .map(|_| rng.gauss() as f32 * 0.003)
        .collect()
}

/// POCSAG NRZ audio (what an FM discriminator would output).
pub fn pocsag_audio(rate: u32, addr: u32, content: pocsag::Content, amp: f32) -> Vec<f32> {
    let bits = pocsag::transmission_bits(addr, 0, content);
    let sps = f64::from(rate) / 1200.0;
    let n = (bits.len() as f64 * sps).ceil() as usize;
    let mut lpf = 0.0f32;
    (0..n)
        .map(|i| {
            let bit = bits[((i as f64 / sps) as usize).min(bits.len() - 1)];
            let lvl = if bit { -amp } else { amp };
            lpf += 0.35 * (lvl - lpf);
            lpf
        })
        .collect()
}

fn run_audio_pocsag(args: &SimArgs) -> Result<()> {
    let rate = if args.rate == 0 { 22_050 } else { args.rate };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut th = Throttle::new(f64::from(rate), args.fast);
    let mut rng = Rng::new(args.seed);
    let mut n = 0u32;
    loop {
        n += 1;
        let text = format!("{} #{n}", args.message);
        let audio = pocsag_audio(rate, args.address, pocsag::Content::Alpha(&text), 0.8);
        if !write_audio(&mut out, &mut th, &audio)? {
            return Ok(());
        }
        if args.count > 0 && n >= args.count {
            // trailing pad so decoders flush their last batch
            let pad = gap(&mut rng, rate, 0.4);
            let _ = write_audio(&mut out, &mut th, &pad)?;
            return Ok(());
        }
        let g = gap(&mut rng, rate, 3.0);
        if !write_audio(&mut out, &mut th, &g)? {
            return Ok(());
        }
    }
}

/// AFSK1200 audio for an AX.25 frame (Bell 202, continuous phase).
pub fn afsk_audio(rate: u32, frame: &[u8], amp: f32) -> Vec<f32> {
    let levels = ax25::nrzi(&ax25::hdlc_bits(frame, 24, 3));
    let sps = f64::from(rate) / 1200.0;
    let n = (levels.len() as f64 * sps).ceil() as usize;
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let mark = levels[((i as f64 / sps) as usize).min(levels.len() - 1)];
            let f = if mark { 1200.0 } else { 2200.0 };
            ph += 2.0 * std::f32::consts::PI * f / rate as f32;
            if ph > std::f32::consts::PI {
                ph -= 2.0 * std::f32::consts::PI;
            }
            ph.sin() * amp
        })
        .collect()
}

fn run_audio_aprs(args: &SimArgs) -> Result<()> {
    let rate = if args.rate == 0 { 22_050 } else { args.rate };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut th = Throttle::new(f64::from(rate), args.fast);
    let mut rng = Rng::new(args.seed);
    let mut n = 0u32;
    loop {
        n += 1;
        let info = match n % 3 {
            0 => format!(">{} #{n}", args.message),
            1 => "!4447.40N/02027.60E>073/036 deck mobile".to_string(),
            _ => format!("T#{:03},123,045,678,010,090,00000000", n % 1000),
        };
        let frame = ax25::frame_bytes(&args.callsign, "APDECK", &["WIDE1-1"], &info);
        let audio = afsk_audio(rate, &frame, 0.75);
        if !write_audio(&mut out, &mut th, &audio)? {
            return Ok(());
        }
        if args.count > 0 && n >= args.count {
            let pad = gap(&mut rng, rate, 0.4);
            let _ = write_audio(&mut out, &mut th, &pad)?;
            return Ok(());
        }
        let g = gap(&mut rng, rate, 2.2);
        if !write_audio(&mut out, &mut th, &g)? {
            return Ok(());
        }
    }
}

/// RTTY audio: 45.45 Bd Baudot on 1585/1415 Hz (minimodem's defaults).
pub fn rtty_audio(rate: u32, text: &str, amp: f32) -> Vec<f32> {
    let bits = baudot::encode(text);
    let sps = f64::from(rate) / 45.45;
    let n = (bits.len() as f64 * sps).ceil() as usize;
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let mark = bits[((i as f64 / sps) as usize).min(bits.len() - 1)];
            let f = if mark { 1585.0 } else { 1415.0 };
            ph += 2.0 * std::f32::consts::PI * f / rate as f32;
            if ph > std::f32::consts::PI {
                ph -= 2.0 * std::f32::consts::PI;
            }
            ph.sin() * amp
        })
        .collect()
}

fn run_audio_rtty(args: &SimArgs) -> Result<()> {
    let rate = if args.rate == 0 { 22_050 } else { args.rate };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut th = Throttle::new(f64::from(rate), args.fast);
    let mut rng = Rng::new(args.seed);
    let mut n = 0u32;
    loop {
        n += 1;
        let audio = rtty_audio(
            rate,
            &format!("RYRYRY DE DECK {} NR {n} ", args.message),
            0.75,
        );
        if !write_audio(&mut out, &mut th, &audio)? {
            return Ok(());
        }
        if args.count > 0 && n >= args.count {
            let pad = gap(&mut rng, rate, 0.4);
            let _ = write_audio(&mut out, &mut th, &pad)?;
            return Ok(());
        }
        let g = gap(&mut rng, rate, 1.2);
        if !write_audio(&mut out, &mut th, &g)? {
            return Ok(());
        }
    }
}

// ------------------------------------------------------------ decoded lines

fn sleep_ms(ms: u64, fast: bool) {
    if !fast && ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

fn run_lines(args: &SimArgs) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut rng = Rng::new(args.seed);
    let mut n = 0u32;
    match args.mode.as_str() {
        "adsb" => {
            let mut fleet = adsb::Fleet::new(args.seed, 6, 44.8, 20.3);
            loop {
                n += 1;
                for line in fleet.step(1.0) {
                    if !out_ok(writeln!(out, "{line}"))? {
                        return Ok(());
                    }
                }
                out.flush().ok();
                if args.count > 0 && n >= args.count {
                    return Ok(());
                }
                sleep_ms(1000, args.fast);
            }
        }
        "dmr" | "ysf" | "dstar" | "nxdn" | "p25" | "m17" => {
            let mut sim = voice::VoiceSim::new(&args.mode, args.seed);
            loop {
                n += 1;
                for l in sim.call() {
                    sleep_ms(l.delay_ms, args.fast);
                    if !out_ok(writeln!(out, "{}", l.text))? {
                        return Ok(());
                    }
                    out.flush().ok();
                }
                if args.count > 0 && n >= args.count {
                    return Ok(());
                }
                sleep_ms(sim.idle_gap_ms(), args.fast);
            }
        }
        "pocsag" => loop {
            n += 1;
            let line = match n % 4 {
                3 => format!(
                    "POCSAG512: Address: {:7}  Function: 3  Numeric: 06{}",
                    args.address % 2_097_152,
                    10_000_000 + (rng.next_u64() % 89_999_999)
                ),
                _ => format!(
                    "POCSAG1200: Address: {:7}  Function: 0  Alpha:   {} #{n}",
                    args.address + n % 5,
                    args.message
                ),
            };
            if !out_ok(writeln!(out, "{line}"))? {
                return Ok(());
            }
            out.flush().ok();
            if args.count > 0 && n >= args.count {
                return Ok(());
            }
            sleep_ms(u64::from(rng.range_u32(1500, 5000)), args.fast);
        },
        "aprs" => loop {
            n += 1;
            let info = match n % 3 {
                0 => format!(">{} #{n}", args.message),
                1 => "!4447.40N/02027.60E>073/036 deck mobile".to_string(),
                _ => format!("T#{:03},123,045,678,010,090,00000000", n % 1000),
            };
            let h = format!(
                "AFSK1200: fm {} to APDECK-0 via WIDE1-1,WIDE2-2 UI pid=F0",
                args.callsign
            );
            if !out_ok(writeln!(out, "{h}"))? || !out_ok(writeln!(out, "{info}"))? {
                return Ok(());
            }
            out.flush().ok();
            if args.count > 0 && n >= args.count {
                return Ok(());
            }
            sleep_ms(u64::from(rng.range_u32(1200, 4200)), args.fast);
        },
        "rtty" => loop {
            n += 1;
            if !out_ok(writeln!(out, "RYRYRY DE DECK {} NR {n}", args.message))? {
                return Ok(());
            }
            out.flush().ok();
            if args.count > 0 && n >= args.count {
                return Ok(());
            }
            sleep_ms(1800, args.fast);
        },
        other => {
            if !out_ok(writeln!(
                out,
                "deck simgen: no line feed for '{other}' — it simulates at IQ level"
            ))? {
                return Ok(());
            }
            Ok(())
        }
    }
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Slice audio into bits at a fixed baud (we generated it, phase = 0).
    fn slice_bits(audio: &[f32], rate: u32, baud: f64) -> Vec<bool> {
        let sps = f64::from(rate) / baud;
        let n = (audio.len() as f64 / sps) as usize;
        (0..n)
            .map(|i| audio[((i as f64 + 0.5) * sps) as usize] < 0.0)
            .collect()
    }

    #[test]
    fn pocsag_audio_decodes_back() {
        let rate = 22_050;
        let audio = pocsag_audio(rate, 1_234_567, pocsag::Content::Alpha("HELLO DECK"), 0.8);
        let bits = slice_bits(&audio, rate, 1200.0);
        // find the sync word
        let mut w = 0u32;
        let mut sync_at = None;
        for (i, b) in bits.iter().enumerate() {
            w = w << 1 | u32::from(*b);
            if i >= 31 && w == pocsag::SYNC {
                sync_at = Some(i + 1);
                break;
            }
        }
        let start = sync_at.expect("sync found");
        // word k of the whole transmission; each batch of 16 words is
        // preceded by its own sync word
        let word_at = |k: usize| -> u32 {
            let batch = k / 16;
            let off = start + k * 32 + batch * 32;
            bits[off..off + 32]
                .iter()
                .fold(0, |acc, b| acc << 1 | u32::from(*b))
        };
        // the second batch must start with sync
        let sync2 = bits[start + 512..start + 512 + 32]
            .iter()
            .fold(0u32, |acc, b| acc << 1 | u32::from(*b));
        assert_eq!(sync2, pocsag::SYNC, "batch 2 sync");
        let mut addr = None;
        let mut text_bits: Vec<bool> = Vec::new();
        for k in 0..32 {
            let cw = word_at(k);
            assert_eq!(pocsag::syndrome(cw), 0, "codeword {k} corrupt");
            if cw == pocsag::IDLE {
                continue;
            }
            if cw >> 31 == 0 {
                addr = Some(((cw >> 13) & 0x3_FFFF) << 3 | (k as u32 % 16) / 2);
            } else {
                for i in (11..31).rev() {
                    text_bits.push((cw >> i) & 1 == 1);
                }
            }
        }
        assert_eq!(addr, Some(1_234_567));
        let mut text = String::new();
        for ch in text_bits.chunks(7) {
            if ch.len() < 7 {
                break;
            }
            let mut v = 0u8;
            for (i, b) in ch.iter().enumerate() {
                if *b {
                    v |= 1 << i;
                }
            }
            if v == 0 {
                break;
            }
            text.push(v as char);
        }
        assert_eq!(text, "HELLO DECK");
    }

    /// Goertzel-style tone classification per bit window.
    fn classify_tone(win: &[f32], rate: f32, f_a: f32, f_b: f32) -> bool {
        let power = |f: f32| {
            let (mut c, mut s) = (0.0f32, 0.0f32);
            for (i, x) in win.iter().enumerate() {
                let w = 2.0 * std::f32::consts::PI * f * i as f32 / rate;
                c += x * w.cos();
                s += x * w.sin();
            }
            c * c + s * s
        };
        power(f_a) > power(f_b)
    }

    #[test]
    fn aprs_audio_demodulates_back() {
        let rate = 22_050u32;
        let frame = ax25::frame_bytes("N0DECK-9", "APDECK", &["WIDE1-1"], ">deck e2e test");
        let audio = afsk_audio(rate, &frame, 0.75);
        let sps = f64::from(rate) / 1200.0;
        let nbits = (audio.len() as f64 / sps) as usize;
        let levels: Vec<bool> = (0..nbits)
            .map(|i| {
                let a = (i as f64 * sps) as usize;
                let b = ((i as f64 + 1.0) * sps) as usize;
                classify_tone(&audio[a..b.min(audio.len())], rate as f32, 1200.0, 2200.0)
            })
            .collect();
        let bits = ax25::hdlc_rx::nrzi_decode(&levels);
        let recovered = ax25::hdlc_rx::extract_frame(&bits).expect("frame from audio");
        assert_eq!(ax25::hdlc_rx::decode_call(&recovered[7..14]), "N0DECK-9");
        assert_eq!(&recovered[23..], b">deck e2e test");
    }

    #[test]
    fn rtty_audio_decodes_back() {
        let rate = 22_050u32;
        let text = "CQ CQ DE DECK 599";
        let audio = rtty_audio(rate, text, 0.75);
        let sps = f64::from(rate) / 45.45;
        let nbits = (audio.len() as f64 / sps) as usize;
        let bits: Vec<bool> = (0..nbits)
            .map(|i| {
                let a = (i as f64 * sps) as usize;
                let b = ((i as f64 + 1.0) * sps) as usize;
                classify_tone(&audio[a..b.min(audio.len())], rate as f32, 1585.0, 1415.0)
            })
            .collect();
        assert_eq!(baudot::decode(&bits), text);
    }
}
