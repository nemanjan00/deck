//! Shared DSP: seeded RNG, FFT spectrum helper, spectral noise reduction.

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// xorshift64* — tiny, deterministic, good enough for signal simulation.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// uniform in [0,1)
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// uniform in [lo,hi)
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.f64() * (hi - lo)
    }

    pub fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next_u64() % u64::from(hi - lo)) as u32
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next_u64() % xs.len() as u64) as usize]
    }

    /// approx gaussian (sum of 4 uniforms), zero-mean, ~unit variance
    pub fn gauss(&mut self) -> f64 {
        ((self.f64() + self.f64() + self.f64() + self.f64()) - 2.0) * 1.732
    }
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / n as f32;
            x.sin() * x.sin()
        })
        .collect()
}

/// Windowed magnitude spectrum in dBFS-ish units, reused across frames.
pub struct SpectrumFft {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    pub size: usize,
}

impl SpectrumFft {
    pub fn new(size: usize) -> Self {
        let mut planner = FftPlanner::new();
        Self {
            fft: planner.plan_fft_forward(size),
            window: hann(size),
            size,
        }
    }

    /// Real input (audio). Returns `size/2` dB magnitudes (DC..Nyquist).
    pub fn real_db(&self, samples: &[f32]) -> Vec<f32> {
        let n = self.size;
        let mut buf: Vec<Complex32> = (0..n)
            .map(|i| Complex32::new(samples.get(i).copied().unwrap_or(0.0) * self.window[i], 0.0))
            .collect();
        self.fft.process(&mut buf);
        let norm = 2.0 / n as f32;
        buf[..n / 2]
            .iter()
            .map(|c| 20.0 * (c.norm() * norm + 1e-9).log10())
            .collect()
    }

    /// Complex IQ input. Returns `size` dB magnitudes, fft-shifted so the
    /// center frequency sits in the middle (waterfall convention).
    pub fn iq_db(&self, iq: &[Complex32]) -> Vec<f32> {
        let n = self.size;
        let mut buf: Vec<Complex32> = (0..n)
            .map(|i| iq.get(i).copied().unwrap_or_default() * self.window[i])
            .collect();
        self.fft.process(&mut buf);
        let norm = 1.0 / n as f32;
        let db: Vec<f32> = buf
            .iter()
            .map(|c| 20.0 * (c.norm() * norm + 1e-9).log10())
            .collect();
        let mut out = Vec::with_capacity(n);
        out.extend_from_slice(&db[n / 2..]);
        out.extend_from_slice(&db[..n / 2]);
        out
    }
}

/// Spectral-subtraction noise reduction with overlap-add (frame 512, hop 256).
/// Noise floor per bin tracks a slow-rising minimum; gain floor keeps it from
/// sounding underwater. Latency ≈ one hop (~12 ms @ 22.05 kHz).
pub struct SpectralNr {
    fwd: Arc<dyn Fft<f32>>,
    inv: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    noise: Vec<f32>,   // per-bin noise magnitude estimate
    overlap: Vec<f32>, // output overlap-add tail
    inbuf: Vec<f32>,   // pending input samples
    pub strength: f32, // over-subtraction factor (0 = bypass)
    frames_seen: u64,
}

const NR_N: usize = 512;
const NR_HOP: usize = 256;

impl SpectralNr {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        Self {
            fwd: planner.plan_fft_forward(NR_N),
            inv: planner.plan_fft_inverse(NR_N),
            window: hann(NR_N),
            noise: vec![1e-4; NR_N / 2 + 1],
            overlap: vec![0.0; NR_N],
            inbuf: Vec::with_capacity(NR_N * 2),
            strength: 0.0,
            frames_seen: 0,
        }
    }

    pub fn reset(&mut self) {
        self.noise.iter_mut().for_each(|x| *x = 1e-4);
        self.overlap.iter_mut().for_each(|x| *x = 0.0);
        self.inbuf.clear();
        self.frames_seen = 0;
    }

    /// Process a chunk of f32 samples; returns processed samples (same rate,
    /// possibly different length due to hop buffering).
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.strength <= 0.0 {
            return input.to_vec();
        }
        self.inbuf.extend_from_slice(input);
        let mut out = Vec::with_capacity(input.len());
        while self.inbuf.len() >= NR_N {
            let frame: Vec<f32> = self.inbuf[..NR_N].to_vec();
            self.inbuf.drain(..NR_HOP);
            out.extend_from_slice(&self.frame(&frame));
        }
        out
    }

    fn frame(&mut self, frame: &[f32]) -> Vec<f32> {
        let mut buf: Vec<Complex32> = frame
            .iter()
            .zip(&self.window)
            .map(|(s, w)| Complex32::new(s * w, 0.0))
            .collect();
        self.fwd.process(&mut buf);
        self.frames_seen += 1;

        let half = NR_N / 2 + 1;
        for k in 0..half {
            let mag = buf[k].norm();
            let n = &mut self.noise[k];
            // slow-rising minimum tracker: drop fast, rise very slowly
            if mag < *n || self.frames_seen < 20 {
                *n = 0.9 * *n + 0.1 * mag;
            } else {
                *n *= 1.008;
            }
            let sub = self.strength * *n;
            let gain = ((mag - sub).max(0.12 * mag)) / (mag + 1e-9);
            buf[k] *= gain;
            if k > 0 && k < NR_N / 2 {
                buf[NR_N - k] *= gain; // mirror for real signal
            }
        }

        self.inv.process(&mut buf);
        let scale = 1.0 / NR_N as f32;
        // overlap-add with synthesis window; hann + 50% overlap sums to 1
        let mut y = vec![0.0f32; NR_HOP];
        for i in 0..NR_N {
            let v = buf[i].re * scale * self.window[i] * (2.0 / 1.0);
            self.overlap[i] += v;
        }
        y.copy_from_slice(&self.overlap[..NR_HOP]);
        self.overlap.copy_within(NR_HOP.., 0);
        self.overlap[NR_N - NR_HOP..].iter_mut().for_each(|x| *x = 0.0);
        y
    }
}

impl Default for SpectralNr {
    fn default() -> Self {
        Self::new()
    }
}

/// RBJ biquad — building block for the customizable analog audio filters.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn lowpass(fs: f32, fc: f32, q: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * (fc / fs).min(0.49);
        let (sw, cw) = (w.sin(), w.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cw) / 2.0) / a0,
            b1: (1.0 - cw) / a0,
            b2: ((1.0 - cw) / 2.0) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn highpass(fs: f32, fc: f32, q: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * (fc / fs).min(0.49);
        let (sw, cw) = (w.sin(), w.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cw) / 2.0) / a0,
            b1: -(1.0 + cw) / a0,
            b2: ((1.0 + cw) / 2.0) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        // transposed direct form II
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn process(&mut self, xs: &mut [f32]) {
        for x in xs {
            *x = self.tick(*x);
        }
    }
}

/// The user-adjustable analog audio filter chain: HP + LP, two biquads each
/// (24 dB/oct), cutoffs picked from preset ladders. 0 = filter off.
pub struct FilterChain {
    fs: f32,
    pub hp_hz: u32,
    pub lp_hz: u32,
    hp: [Biquad; 2],
    lp: [Biquad; 2],
}

pub const HP_LADDER: &[u32] = &[0, 100, 200, 300, 500];
pub const LP_LADDER: &[u32] = &[0, 2000, 2400, 3000, 3600, 4500, 6000, 8000];

impl FilterChain {
    pub fn new(fs: u32) -> Self {
        Self {
            fs: fs as f32,
            hp_hz: 0,
            lp_hz: 0,
            hp: [Biquad::default(); 2],
            lp: [Biquad::default(); 2],
        }
    }

    pub fn set(&mut self, hp_hz: u32, lp_hz: u32) {
        self.hp_hz = hp_hz;
        self.lp_hz = lp_hz.min((self.fs / 2.0) as u32);
        if self.hp_hz > 0 {
            self.hp = [Biquad::highpass(self.fs, self.hp_hz as f32, 0.707); 2];
        }
        if self.lp_hz > 0 {
            self.lp = [Biquad::lowpass(self.fs, self.lp_hz as f32, 0.707); 2];
        }
    }

    pub fn process(&mut self, xs: &mut [f32]) {
        if self.hp_hz > 0 {
            for bq in &mut self.hp {
                bq.process(xs);
            }
        }
        if self.lp_hz > 0 {
            for bq in &mut self.lp {
                bq.process(xs);
            }
        }
    }
}

// ───────────────────────────── radio DSP ─────────────────────────────
// deck's internal receiver: NCO tuning, FIR decimation, FM/AM/SSB demod.

/// Numerically controlled oscillator: complex frequency shift.
pub struct Nco {
    phase: Complex32,
    step: Complex32,
    n: u32,
}

impl Nco {
    pub fn new(freq_hz: f64, fs: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * freq_hz / fs;
        Self {
            phase: Complex32::new(1.0, 0.0),
            step: Complex32::new(w.cos() as f32, w.sin() as f32),
            n: 0,
        }
    }

    pub fn set_freq(&mut self, freq_hz: f64, fs: f64) {
        let w = 2.0 * std::f64::consts::PI * freq_hz / fs;
        self.step = Complex32::new(w.cos() as f32, w.sin() as f32);
    }

    /// Multiply the buffer by e^{j2πft} in place.
    pub fn mix(&mut self, buf: &mut [Complex32]) {
        for x in buf {
            *x *= self.phase;
            self.phase *= self.step;
            self.n += 1;
            if self.n >= 4096 {
                // renormalize to kill accumulated rounding drift
                let m = self.phase.norm();
                if m > 0.0 {
                    self.phase /= m;
                }
                self.n = 0;
            }
        }
    }

    /// Next oscillator sample (for modulators).
    pub fn next(&mut self) -> Complex32 {
        let out = self.phase;
        self.phase *= self.step;
        self.n += 1;
        if self.n >= 4096 {
            let m = self.phase.norm();
            if m > 0.0 {
                self.phase /= m;
            }
            self.n = 0;
        }
        out
    }
}

fn windowed_sinc(taps: usize, cutoff: f32) -> Vec<f32> {
    // cutoff as fraction of the sample rate (0..0.5), Hamming window
    let m = taps - 1;
    let mut h: Vec<f32> = (0..taps)
        .map(|i| {
            let x = i as f32 - m as f32 / 2.0;
            let sinc = if x.abs() < 1e-6 {
                2.0 * cutoff
            } else {
                (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
            };
            let w = 0.54
                - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / m as f32).cos();
            sinc * w
        })
        .collect();
    let sum: f32 = h.iter().sum();
    h.iter_mut().for_each(|v| *v /= sum);
    h
}

/// FIR low-pass + integer decimator over complex samples.
pub struct FirDecim {
    taps: Vec<f32>,
    hist: Vec<Complex32>,
    pub factor: usize,
    phase: usize,
}

impl FirDecim {
    pub fn new(factor: usize) -> Self {
        let taps = windowed_sinc(factor * 8 + 1, 0.42 / factor as f32);
        Self {
            hist: vec![Complex32::default(); taps.len()],
            taps,
            factor,
            phase: 0,
        }
    }

    pub fn process(&mut self, input: &[Complex32], out: &mut Vec<Complex32>) {
        let tl = self.taps.len();
        for &x in input {
            self.hist.copy_within(1.., 0);
            self.hist[tl - 1] = x;
            self.phase += 1;
            if self.phase >= self.factor {
                self.phase = 0;
                let mut acc = Complex32::default();
                for (h, t) in self.hist.iter().zip(&self.taps) {
                    acc += h * t;
                }
                out.push(acc);
            }
        }
    }
}

/// FIR low-pass + integer decimator over real samples (WFM audio stage).
pub struct FirDecimF32 {
    taps: Vec<f32>,
    hist: Vec<f32>,
    pub factor: usize,
    phase: usize,
}

impl FirDecimF32 {
    pub fn new(factor: usize) -> Self {
        let taps = windowed_sinc(factor * 8 + 1, 0.42 / factor as f32);
        Self {
            hist: vec![0.0; taps.len()],
            taps,
            factor,
            phase: 0,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let tl = self.taps.len();
        for &x in input {
            self.hist.copy_within(1.., 0);
            self.hist[tl - 1] = x;
            self.phase += 1;
            if self.phase >= self.factor {
                self.phase = 0;
                let mut acc = 0.0f32;
                for (h, t) in self.hist.iter().zip(&self.taps) {
                    acc += h * t;
                }
                out.push(acc);
            }
        }
    }
}

/// Pick integer decimation factors from `in_rate` down to exactly `target`.
pub fn decim_factors(in_rate: u32, target: u32) -> Option<Vec<usize>> {
    let mut rate = in_rate;
    let mut out = Vec::new();
    while rate > target {
        let mut picked = 0;
        for f in [5usize, 4, 3, 2] {
            if rate % (f as u32) == 0 && rate / (f as u32) >= target {
                picked = f;
                break;
            }
        }
        if picked == 0 {
            return None;
        }
        rate /= picked as u32;
        out.push(picked);
    }
    (rate == target).then_some(out)
}

/// Linear-interpolation resampler (mono f32) — decoder rate adaptation.
pub struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            step: from as f64 / to as f64,
            pos: 0.0,
            prev: 0.0,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &x in input {
            while self.pos < 1.0 {
                out.push(self.prev + (x - self.prev) * self.pos as f32);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = x;
        }
    }
}

/// Quadrature FM discriminator.
pub struct FmDemod {
    prev: Complex32,
    gain: f32,
}

impl FmDemod {
    /// `dev` = expected peak deviation in Hz — output scaled to ±1 there.
    pub fn new(fs: f32, dev: f32) -> Self {
        Self {
            prev: Complex32::new(1.0, 0.0),
            gain: fs / (2.0 * std::f32::consts::PI * dev),
        }
    }

    pub fn process(&mut self, input: &[Complex32], out: &mut Vec<f32>) {
        for &x in input {
            let d = x * self.prev.conj();
            self.prev = x;
            out.push(d.im.atan2(d.re) * self.gain);
        }
    }
}

/// One-pole de-emphasis (WFM).
pub struct Deemphasis {
    a: f32,
    y: f32,
}

impl Deemphasis {
    pub fn new(fs: f32, tau: f32) -> Self {
        Self {
            a: 1.0 - (-1.0 / (fs * tau)).exp(),
            y: 0.0,
        }
    }

    pub fn process(&mut self, xs: &mut [f32]) {
        for x in xs {
            self.y += self.a * (*x - self.y);
            *x = self.y;
        }
    }
}

/// DC blocker (AM envelope).
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    pub fn new() -> Self {
        Self { x1: 0.0, y1: 0.0 }
    }

    pub fn process(&mut self, xs: &mut [f32]) {
        for x in xs {
            let y = *x - self.x1 + 0.995 * self.y1;
            self.x1 = *x;
            self.y1 = y;
            *x = y;
        }
    }
}

impl Default for DcBlock {
    fn default() -> Self {
        Self::new()
    }
}

/// Block AGC with slewed gain (AM/SSB).
pub struct Agc {
    gain: f32,
    target: f32,
}

impl Agc {
    pub fn new(target: f32) -> Self {
        Self { gain: 1.0, target }
    }

    pub fn process(&mut self, xs: &mut [f32]) {
        let peak = xs.iter().fold(1e-4f32, |a, x| a.max(x.abs()));
        let want = (self.target / peak).clamp(0.001, 300.0);
        for x in xs {
            self.gain += (want - self.gain) * 0.002;
            *x = (*x * self.gain).clamp(-1.0, 1.0);
        }
    }
}

/// IQ-domain impulse noise blanker: samples whose magnitude spikes far above
/// the running average get blanked (held at the previous good value).
pub struct NoiseBlanker {
    avg: f32,
    last_good: Complex32,
    /// threshold multiplier; 0 = off (typ. 4.0 gentle, 2.5 aggressive)
    pub factor: f32,
}

impl NoiseBlanker {
    pub fn new() -> Self {
        Self {
            avg: 0.0,
            last_good: Complex32::default(),
            factor: 0.0,
        }
    }

    pub fn process(&mut self, xs: &mut [Complex32]) {
        if self.factor <= 0.0 {
            return;
        }
        for x in xs {
            let m = x.norm();
            self.avg += (m - self.avg) * 0.005;
            if self.avg > 1e-6 && m > self.avg * self.factor {
                *x = self.last_good;
            } else {
                self.last_good = *x;
            }
        }
    }
}

impl Default for NoiseBlanker {
    fn default() -> Self {
        Self::new()
    }
}

/// LMS adaptive auto-notch (adaptive line enhancer): predicts the tonal
/// (periodic) component from a delayed copy and subtracts it. Heterodyne
/// whistles vanish; speech/noise pass through.
pub struct AutoNotch {
    w: Vec<f32>,
    buf: Vec<f32>,
    pos: usize,
    mu: f32,
    pub enabled: bool,
}

const NOTCH_TAPS: usize = 48;
const NOTCH_DELAY: usize = 8;

impl AutoNotch {
    pub fn new() -> Self {
        Self {
            w: vec![0.0; NOTCH_TAPS],
            buf: vec![0.0; NOTCH_TAPS + NOTCH_DELAY],
            pos: 0,
            mu: 0.002,
            enabled: false,
        }
    }

    pub fn process(&mut self, xs: &mut [f32]) {
        if !self.enabled {
            return;
        }
        let blen = self.buf.len();
        for x in xs {
            // predicted tonal component from delayed history
            let mut yhat = 0.0f32;
            let mut power = 1e-4f32;
            for (i, wi) in self.w.iter().enumerate() {
                let s = self.buf[(self.pos + blen - NOTCH_DELAY - i) % blen];
                yhat += wi * s;
                power += s * s;
            }
            let e = *x - yhat;
            // normalized LMS update
            let g = self.mu * e / power;
            for (i, wi) in self.w.iter_mut().enumerate() {
                let s = self.buf[(self.pos + blen - NOTCH_DELAY - i) % blen];
                *wi += g * s;
            }
            self.buf[self.pos] = *x;
            self.pos = (self.pos + 1) % blen;
            *x = e;
        }
    }
}

impl Default for AutoNotch {
    fn default() -> Self {
        Self::new()
    }
}

/// Synchronous AM (SAM): a PLL locks to the carrier and derotates; fades
/// that destroy the envelope leave the sync product usable.
pub struct SamDemod {
    phase: f32,
    freq: f32,
    dc: DcBlock,
    agc: Agc,
}

impl SamDemod {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            freq: 0.0,
            dc: DcBlock::new(),
            agc: Agc::new(0.5),
        }
    }

    pub fn process(&mut self, input: &[Complex32], out: &mut Vec<f32>) {
        let start = out.len();
        for &x in input {
            let osc = Complex32::new(self.phase.cos(), -self.phase.sin());
            let v = x * osc;
            let err = v.im.atan2(v.re.abs().max(1e-6));
            // 2nd-order loop, ~±500 Hz capture at 48k
            self.freq = (self.freq + 0.0008 * err).clamp(-0.07, 0.07);
            self.phase += self.freq + 0.02 * err;
            if self.phase > std::f32::consts::PI {
                self.phase -= 2.0 * std::f32::consts::PI;
            } else if self.phase < -std::f32::consts::PI {
                self.phase += 2.0 * std::f32::consts::PI;
            }
            out.push(v.re);
        }
        self.dc.process(&mut out[start..]);
        self.agc.process(&mut out[start..]);
    }
}

impl Default for SamDemod {
    fn default() -> Self {
        Self::new()
    }
}

/// Weaver-style SSB demodulator at 48 kHz complex baseband.
/// Carrier is at DC (you tune to the suppressed carrier); the wanted
/// sideband is shifted to be symmetric around 0, low-passed, shifted back.
pub struct SsbDemod {
    down: Nco,
    up: Nco,
    lp_i: [Biquad; 3],
    lp_q: [Biquad; 3],
    agc: Agc,
    usb: bool,
}

const SSB_CENTER: f64 = 1650.0; // sideband mid-point (300..3000 Hz)
const SSB_HALF_BW: f32 = 1400.0;

impl SsbDemod {
    pub fn new(fs: f32, usb: bool) -> Self {
        let sign = if usb { -1.0 } else { 1.0 };
        let lp = Biquad::lowpass(fs, SSB_HALF_BW, 0.707);
        Self {
            down: Nco::new(sign * SSB_CENTER, fs as f64),
            up: Nco::new(-sign * SSB_CENTER, fs as f64),
            lp_i: [lp; 3],
            lp_q: [lp; 3],
            agc: Agc::new(0.5),
            usb,
        }
    }

    pub fn process(&mut self, input: &[Complex32], out: &mut Vec<f32>) {
        let start = out.len();
        for &x in input {
            let mut v = x * self.down.next();
            let mut i = v.re;
            let mut q = v.im;
            for k in 0..3 {
                i = self.lp_i[k].tick(i);
                q = self.lp_q[k].tick(q);
            }
            v = Complex32::new(i, q) * self.up.next();
            out.push(v.re * 2.0);
        }
        self.agc.process(&mut out[start..]);
        let _ = self.usb;
    }
}

/// s16le bytes → f32 samples in [-1,1)
pub fn s16_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

/// f32 samples → s16le bytes (clamped)
pub fn f32_to_s16(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = Rng::new(43);
        assert_ne!(a.next_u64(), c.next_u64());
    }

    #[test]
    fn spectrum_finds_tone() {
        let fft = SpectrumFft::new(512);
        let fs = 22050.0;
        let f = 2000.0;
        let samples: Vec<f32> = (0..512)
            .map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / fs).sin() * 0.5)
            .collect();
        let db = fft.real_db(&samples);
        let peak_bin = db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let expect = (f / fs * 512.0).round() as usize;
        assert!((peak_bin as i64 - expect as i64).abs() <= 1);
    }

    #[test]
    fn s16_roundtrip() {
        let s = vec![0.0f32, 0.5, -0.5, 0.999];
        let b = f32_to_s16(&s);
        let s2 = s16_to_f32(&b);
        for (a, b) in s.iter().zip(&s2) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    fn dominant_freq(samples: &[f32], fs: f32) -> f32 {
        let fft = SpectrumFft::new(2048);
        let db = fft.real_db(&samples[samples.len().saturating_sub(2048)..]);
        let bin = db[3..]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0
            + 3;
        bin as f32 * fs / 2048.0
    }

    #[test]
    fn decim_factor_plans() {
        assert_eq!(decim_factors(2_400_000, 48_000), Some(vec![5, 5, 2]));
        assert_eq!(decim_factors(768_000, 48_000), Some(vec![4, 4]));
        assert_eq!(decim_factors(2_400_000, 240_000), Some(vec![5, 2]));
        assert_eq!(decim_factors(48_000, 48_000), Some(vec![]));
        assert_eq!(decim_factors(44_100, 48_000), None);
    }

    #[test]
    fn nco_and_decimator_recover_offset_tone() {
        // carrier at +100 kHz in a 2.4 MHz band → NCO shift → decimate to 48k
        let fs = 2_400_000.0f64;
        let mut carrier = Nco::new(100_000.0, fs);
        let iq: Vec<Complex32> = (0..240_000).map(|_| carrier.next() * 0.5).collect();

        let mut shift = Nco::new(-100_000.0 + 1_000.0, fs); // land it at +1 kHz
        let mut shifted = iq.clone();
        shift.mix(&mut shifted);

        let mut cur = shifted;
        for f in decim_factors(2_400_000, 48_000).unwrap() {
            let mut d = FirDecim::new(f);
            let mut out = Vec::new();
            d.process(&cur, &mut out);
            cur = out;
        }
        // measure the complex tone frequency via FM of a rotated constant? —
        // simpler: real part is a 1 kHz cosine
        let re: Vec<f32> = cur.iter().map(|c| c.re).collect();
        let f = dominant_freq(&re, 48_000.0);
        assert!((f - 1_000.0).abs() < 40.0, "got {f} Hz");
    }

    #[test]
    fn fm_demod_recovers_tone() {
        // NBFM-modulate a 1 kHz tone at 48k, demodulate, compare
        let fs = 48_000.0f32;
        let dev = 3_000.0f32;
        let mut phase = 0.0f32;
        let iq: Vec<Complex32> = (0..48_000)
            .map(|i| {
                let m = (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / fs).sin();
                phase += 2.0 * std::f32::consts::PI * dev * m / fs;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();
        let mut demod = FmDemod::new(fs, dev);
        let mut audio = Vec::new();
        demod.process(&iq, &mut audio);
        let f = dominant_freq(&audio, fs);
        assert!((f - 1_000.0).abs() < 40.0, "got {f} Hz");
        let peak = audio[100..].iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!((peak - 1.0).abs() < 0.2, "deviation scaling off: {peak}");
    }

    #[test]
    fn ssb_demod_recovers_tone() {
        // a USB signal: carrier suppressed, single tone at +1.2 kHz
        let fs = 48_000.0f64;
        let mut tone = Nco::new(1_200.0, fs);
        let iq: Vec<Complex32> = (0..96_000).map(|_| tone.next() * 0.4).collect();
        let mut demod = SsbDemod::new(fs as f32, true);
        let mut audio = Vec::new();
        demod.process(&iq, &mut audio);
        let f = dominant_freq(&audio, fs as f32);
        assert!((f - 1_200.0).abs() < 50.0, "usb got {f} Hz");

        // LSB demod of the same (positive-frequency) tone should reject it
        let mut lsb = SsbDemod::new(fs as f32, false);
        let mut rej = Vec::new();
        lsb.process(&iq, &mut rej);
        let usb_pow: f32 = audio[4096..].iter().map(|x| x * x).sum();
        let lsb_pow: f32 = rej[4096..].iter().map(|x| x * x).sum();
        assert!(
            lsb_pow < usb_pow * 0.15,
            "lsb should reject usb tone (usb {usb_pow}, lsb {lsb_pow})"
        );
    }

    #[test]
    fn noise_blanker_kills_impulses() {
        let mut rng = Rng::new(11);
        let mut iq: Vec<Complex32> = (0..8000)
            .map(|_| Complex32::new(rng.gauss() as f32 * 0.05, rng.gauss() as f32 * 0.05))
            .collect();
        for i in (500..8000).step_by(500) {
            iq[i] = Complex32::new(3.0, 3.0); // ignition-style spikes
        }
        let mut nb = NoiseBlanker::new();
        nb.factor = 4.0;
        let mut blanked = iq.clone();
        nb.process(&mut blanked);
        let peak_before = iq.iter().map(|c| c.norm()).fold(0.0f32, f32::max);
        let peak_after = blanked[1000..].iter().map(|c| c.norm()).fold(0.0f32, f32::max);
        assert!(peak_before > 4.0);
        assert!(peak_after < 0.5, "spikes should be blanked, got {peak_after}");
    }

    #[test]
    fn auto_notch_removes_tone_keeps_noise() {
        let fs = 48_000.0f32;
        let mut rng = Rng::new(5);
        let mut sig: Vec<f32> = (0..48_000)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 1_800.0 * i as f32 / fs).sin() * 0.4
                    + rng.gauss() as f32 * 0.05
            })
            .collect();
        let mut notch = AutoNotch::new();
        notch.enabled = true;
        notch.process(&mut sig);
        let fft = SpectrumFft::new(2048);
        let db = fft.real_db(&sig[sig.len() - 2048..]);
        let tone_bin = (1_800.0 / fs * 2048.0).round() as usize;
        let tone = db[tone_bin - 1..=tone_bin + 1]
            .iter()
            .fold(f32::MIN, |a, &b| a.max(b));
        let floor: f32 = db[400..500].iter().sum::<f32>() / 100.0;
        assert!(
            tone < floor + 12.0,
            "tone should sink toward the floor (tone {tone:.1} floor {floor:.1})"
        );
    }

    #[test]
    fn sam_demod_recovers_modulation() {
        // AM carrier with 30% mod at 800 Hz, carrier off-frequency by 120 Hz
        let fs = 48_000.0f64;
        let mut carrier = Nco::new(120.0, fs);
        let iq: Vec<Complex32> = (0..96_000)
            .map(|i| {
                let m = 1.0
                    + 0.3 * (2.0 * std::f32::consts::PI * 800.0 * i as f32 / fs as f32).sin();
                carrier.next() * m * 0.5
            })
            .collect();
        let mut sam = SamDemod::new();
        let mut audio = Vec::new();
        sam.process(&iq, &mut audio);
        let f = dominant_freq(&audio[48_000..], fs as f32);
        assert!((f - 800.0).abs() < 40.0, "sam got {f} Hz");
    }

    #[test]
    fn resampler_ratio() {
        let mut r = Resampler::new(48_000, 22_050);
        let input = vec![0.5f32; 48_000];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert!((out.len() as i64 - 22_050).abs() < 20, "{}", out.len());
    }

    #[test]
    fn filters_shape_spectrum() {
        let fs = 22050.0;
        let mut rng = Rng::new(3);
        let noise: Vec<f32> = (0..8192).map(|_| rng.gauss() as f32 * 0.2).collect();

        let mut chain = FilterChain::new(fs as u32);
        chain.set(300, 3000);
        let mut out = noise.clone();
        chain.process(&mut out);

        let fft = SpectrumFft::new(512);
        let before = fft.real_db(&noise[4096..4608]);
        let after = fft.real_db(&out[4096..4608]);
        let bin = |f: f32| (f / fs * 512.0).round() as usize;
        // well inside the stopbands, expect heavy attenuation
        assert!(after[bin(60.0)] < before[bin(60.0)] - 10.0, "HP cuts lows");
        assert!(after[bin(8000.0)] < before[bin(8000.0)] - 20.0, "LP cuts highs");
        // passband roughly intact (allow a few dB)
        assert!(after[bin(1000.0)] > before[bin(1000.0)] - 6.0, "passband kept");
    }

    #[test]
    fn nr_improves_snr() {
        // tone at 1 kHz buried in white noise; NR should raise tone/noise ratio
        let fs = 22050.0;
        let mut rng = Rng::new(7);
        let n = 22050;
        let sig: Vec<f32> = (0..n)
            .map(|i| {
                let t = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / fs).sin() * 0.3;
                t + rng.gauss() as f32 * 0.1
            })
            .collect();

        let band_ratio = |x: &[f32]| {
            let fft = SpectrumFft::new(512);
            let db = fft.real_db(&x[x.len() - 512..]);
            let tone_bin = (1000.0 / fs * 512.0).round() as usize;
            let tone = db[tone_bin - 1..=tone_bin + 1]
                .iter()
                .fold(f32::MIN, |a, &b| a.max(b));
            let noise: f32 =
                db[100..200].iter().sum::<f32>() / 100.0; // 4.3–8.6 kHz: noise only
            tone - noise
        };

        let mut nr = SpectralNr::new();
        nr.strength = 2.0;
        let out = nr.process(&sig);
        assert!(out.len() > 512);
        let before = band_ratio(&sig);
        let after = band_ratio(&out);
        assert!(
            after > before + 3.0,
            "NR should improve tone-to-noise by >3dB (before {before:.1}, after {after:.1})"
        );
    }
}
