//! FT8 decoder output parser. Handles both WSJT-X `jt9`/`ft8code` style and
//! `ft8_lib` `decode_ft8` style lines, which share the shape:
//!
//!   `HHMMSS  SNR  DT  FREQ ~ MESSAGE`   (jt9: `134500  -8  0.3 1500 ~  CQ …`)
//!   `000000 -21  0.5 1000 ~ CQ DL1ABC` (ft8_lib)
//!
//! Tolerant: SNR (signed int), DT (float), FREQ (int Hz), message = the rest
//! after an optional `~`.

#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    pub snr: i32,
    /// time offset (s)
    pub dt: f32,
    /// audio offset frequency (Hz)
    pub freq: u32,
    pub msg: String,
}

pub fn parse_spot(line: &str) -> Option<Spot> {
    let line = line.trim();
    // drop a leading HHMMSS timestamp token if present
    let mut toks: Vec<&str> = line.split_whitespace().collect();
    if toks.is_empty() {
        return None;
    }
    if toks[0].len() == 6 && toks[0].chars().all(|c| c.is_ascii_digit()) {
        toks.remove(0);
    }
    // now: SNR DT FREQ [~] MESSAGE...
    if toks.len() < 4 {
        return None;
    }
    let snr: i32 = toks[0].parse().ok()?;
    let dt: f32 = toks[1].parse().ok()?;
    let freq: u32 = toks[2].parse().ok()?;
    // plausibility: SNR in dB range, freq in the audio passband
    if !(-30..=50).contains(&snr) || freq > 6000 {
        return None;
    }
    let mut rest = &toks[3..];
    if rest.first() == Some(&"~") {
        rest = &rest[1..];
    }
    let msg = rest.join(" ");
    if msg.is_empty() {
        return None;
    }
    Some(Spot { snr, dt, freq, msg })
}

/// Pull the callsign(s) of interest for quick display (best-effort): the
/// second token of a `CQ <call> <grid>` or the last call in a reply.
pub fn callers(msg: &str) -> Option<String> {
    let t: Vec<&str> = msg.split_whitespace().collect();
    match t.as_slice() {
        ["CQ", "DX", call, ..] => Some((*call).to_string()),
        ["CQ", call, ..] => Some((*call).to_string()),
        [_, call, ..] => Some((*call).to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jt9_line() {
        let s = parse_spot("134500  -8  0.3 1500 ~  CQ DL1ABC JO31").unwrap();
        assert_eq!(s.snr, -8);
        assert!((s.dt - 0.3).abs() < 1e-4);
        assert_eq!(s.freq, 1500);
        assert_eq!(s.msg, "CQ DL1ABC JO31");
        assert_eq!(callers(&s.msg).as_deref(), Some("DL1ABC"));
    }

    #[test]
    fn ft8lib_line_no_timestamp() {
        let s = parse_spot("-21  0.5 1000 ~ CQ K1ABC FN42").unwrap();
        assert_eq!(s.snr, -21);
        assert_eq!(s.freq, 1000);
        assert_eq!(s.msg, "CQ K1ABC FN42");
    }

    #[test]
    fn reply_message() {
        let s = parse_spot("000000  2 -0.1  700 ~ YU1ABC DL1ABC -05").unwrap();
        assert_eq!(s.snr, 2);
        assert_eq!(s.msg, "YU1ABC DL1ABC -05");
        assert_eq!(callers(&s.msg).as_deref(), Some("DL1ABC"));
    }

    #[test]
    fn rejects_noise() {
        assert!(parse_spot("").is_none());
        assert!(parse_spot("Decoding...").is_none());
        assert!(parse_spot("134500 -8 0.3").is_none()); // no message
        assert!(parse_spot("134500 999 0.3 1500 ~ x").is_none()); // bad SNR
    }
}
