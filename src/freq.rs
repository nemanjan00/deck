//! Frequency value + radio-style digit editor (10 digits, up to 9.999 GHz).

pub const MAX_HZ: u64 = 9_999_999_999;

/// Radio-style frequency input: a cursor sits on one decimal digit,
/// up/down increments that digit's weight, typing replaces it.
#[derive(Clone, Copy, Debug)]
pub struct FreqInput {
    pub hz: u64,
    /// Digit index under the cursor. 0 = 1 Hz, 3 = 1 kHz, 6 = 1 MHz, 9 = 1 GHz.
    pub cursor: u32,
}

impl FreqInput {
    pub fn new(hz: u64) -> Self {
        Self {
            hz: hz.min(MAX_HZ),
            cursor: 3, // kHz digit: a sane default step for tuning
        }
    }

    fn weight(&self) -> u64 {
        10u64.pow(self.cursor)
    }

    pub fn up(&mut self) {
        let w = self.weight();
        if self.hz + w <= MAX_HZ {
            self.hz += w;
        }
    }

    pub fn down(&mut self) {
        let w = self.weight();
        self.hz = self.hz.saturating_sub(w);
    }

    pub fn left(&mut self) {
        if self.cursor < 9 {
            self.cursor += 1;
        }
    }

    pub fn right(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Type a digit at the cursor, then advance the cursor to the right
    /// (like keying a frequency into a radio head unit).
    pub fn type_digit(&mut self, d: u8) {
        let w = self.weight();
        let cur = (self.hz / w) % 10;
        self.hz = self.hz - cur * w + u64::from(d) * w;
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// The 10 digits, most significant first.
    pub fn digits(&self) -> [u8; 10] {
        let mut out = [0u8; 10];
        let mut v = self.hz;
        for i in (0..10).rev() {
            out[9 - i] = ((v / 10u64.pow(i as u32)) % 10) as u8;
            v %= 10u64.pow(i as u32);
        }
        out
    }
}

/// "145.800.000" style grouping (Hz).
pub fn fmt_hz(hz: u64) -> String {
    let s = hz.to_string();
    let b = s.as_bytes();
    let mut out = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(*c as char);
    }
    out
}

/// "145.800000" MHz with six decimals (what airspyhf_rx and friends want).
pub fn fmt_mhz(hz: u64) -> String {
    format!("{}.{:06}", hz / 1_000_000, hz % 1_000_000)
}

/// Short human form: "145.800 MHz", "7.646 kHz" style, trimmed.
pub fn fmt_short(hz: u64) -> String {
    if hz >= 1_000_000 {
        let mhz = hz as f64 / 1e6;
        let s = format!("{mhz:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        format!("{s} MHz")
    } else if hz >= 1_000 {
        let khz = hz as f64 / 1e3;
        let s = format!("{khz:.3}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        format!("{s} kHz")
    } else {
        format!("{hz} Hz")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_editing() {
        let mut f = FreqInput::new(145_800_000);
        f.cursor = 3; // 1 kHz
        f.up();
        assert_eq!(f.hz, 145_801_000);
        f.down();
        f.down();
        assert_eq!(f.hz, 145_799_000);
        f.cursor = 6; // 1 MHz
        f.type_digit(9);
        assert_eq!(f.hz, 149_799_000);
        assert_eq!(f.cursor, 5); // advanced right
    }

    #[test]
    fn clamps() {
        let mut f = FreqInput::new(MAX_HZ);
        f.cursor = 9;
        f.up(); // must not overflow
        assert_eq!(f.hz, MAX_HZ);
        let mut f = FreqInput::new(500);
        f.cursor = 3;
        f.down();
        assert_eq!(f.hz, 0);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_hz(1_090_000_000), "1.090.000.000");
        assert_eq!(fmt_hz(169_650_000), "169.650.000");
        assert_eq!(fmt_mhz(145_800_000), "145.800000");
        assert_eq!(fmt_short(446_006_250), "446.0063 MHz");
        assert_eq!(fmt_short(145_500_000), "145.5 MHz");
    }

    #[test]
    fn digits_roundtrip() {
        let f = FreqInput::new(1_090_000_000);
        let d = f.digits();
        assert_eq!(d, [1, 0, 9, 0, 0, 0, 0, 0, 0, 0]);
    }
}
