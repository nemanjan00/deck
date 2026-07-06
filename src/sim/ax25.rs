//! AX.25 UI frames + HDLC bit layer (stuffing, NRZI) for AFSK1200/APRS.

/// CRC-16/X-25 (the AX.25 FCS): poly 0x8408 reflected, init/xorout 0xFFFF.
pub fn crc_x25(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF
}

fn push_call(out: &mut Vec<u8>, call: &str, last: bool, cbit: bool) {
    let (base, ssid) = match call.split_once('-') {
        Some((b, s)) => (b, s.parse::<u8>().unwrap_or(0)),
        None => (call, 0),
    };
    let mut chars = base.chars();
    for _ in 0..6 {
        let c = chars.next().unwrap_or(' ').to_ascii_uppercase() as u8;
        out.push(c << 1);
    }
    let mut b = 0x60 | ((ssid & 0x0F) << 1);
    if last {
        b |= 0x01;
    }
    if cbit {
        b |= 0x80;
    }
    out.push(b);
}

/// Frame bytes: DEST SRC [PATH…] 0x03 0xF0 INFO FCS(lo,hi).
pub fn frame_bytes(src: &str, dest: &str, path: &[&str], info: &str) -> Vec<u8> {
    let mut f = Vec::with_capacity(16 + info.len() + path.len() * 7);
    push_call(&mut f, dest, false, true);
    push_call(&mut f, src, path.is_empty(), false);
    for (i, p) in path.iter().enumerate() {
        push_call(&mut f, p, i + 1 == path.len(), false);
    }
    f.push(0x03); // UI
    f.push(0xF0); // no layer 3
    f.extend_from_slice(info.as_bytes());
    let fcs = crc_x25(&f);
    f.push((fcs & 0xFF) as u8);
    f.push((fcs >> 8) as u8);
    f
}

const FLAG_BITS: [bool; 8] = [false, true, true, true, true, true, true, false];

/// HDLC air bits: lead-in flags, stuffed frame (bytes LSB-first), tail flags.
pub fn hdlc_bits(frame: &[u8], lead_flags: usize, tail_flags: usize) -> Vec<bool> {
    let mut bits = Vec::with_capacity(lead_flags * 8 + frame.len() * 10 + tail_flags * 8);
    for _ in 0..lead_flags {
        bits.extend_from_slice(&FLAG_BITS);
    }
    let mut ones = 0u32;
    for &byte in frame {
        for i in 0..8 {
            let b = byte >> i & 1 == 1;
            bits.push(b);
            if b {
                ones += 1;
                if ones == 5 {
                    bits.push(false); // stuff
                    ones = 0;
                }
            } else {
                ones = 0;
            }
        }
    }
    for _ in 0..tail_flags {
        bits.extend_from_slice(&FLAG_BITS);
    }
    bits
}

/// NRZI encode: 0 = toggle, 1 = hold. Returns tone levels (true = mark).
pub fn nrzi(bits: &[bool]) -> Vec<bool> {
    let mut level = true;
    bits.iter()
        .map(|b| {
            if !*b {
                level = !level;
            }
            level
        })
        .collect()
}

#[cfg(test)]
pub mod hdlc_rx {
    //! Test-side HDLC receiver: flag hunt, unstuff, FCS check.

    pub fn nrzi_decode(levels: &[bool]) -> Vec<bool> {
        let mut prev = levels.first().copied().unwrap_or(true);
        let mut out = Vec::with_capacity(levels.len());
        for &l in &levels[1..] {
            out.push(l == prev);
            prev = l;
        }
        out
    }

    /// Extract the first frame with a valid FCS from a raw bitstream.
    pub fn extract_frame(bits: &[bool]) -> Option<Vec<u8>> {
        let flag = [false, true, true, true, true, true, true, false];
        let is_flag_at = |i: usize| bits.len() >= i + 8 && bits[i..i + 8] == flag;
        let mut starts: Vec<usize> = Vec::new();
        for i in 0..bits.len().saturating_sub(8) {
            if is_flag_at(i) {
                starts.push(i);
            }
        }
        for (a, &s) in starts.iter().enumerate() {
            for &e in &starts[a + 1..] {
                if e <= s + 8 {
                    continue;
                }
                // unstuff the span between the flags
                let span = &bits[s + 8..e];
                let mut ones = 0u32;
                let mut out: Vec<bool> = Vec::with_capacity(span.len());
                let mut ok = true;
                let mut skip = false;
                for &b in span {
                    if skip {
                        skip = false;
                        if b {
                            ok = false; // six 1s inside a frame = abort
                            break;
                        }
                        continue;
                    }
                    out.push(b);
                    if b {
                        ones += 1;
                        if ones == 5 {
                            skip = true;
                            ones = 0;
                        }
                    } else {
                        ones = 0;
                    }
                }
                if !ok || out.len() % 8 != 0 || out.len() < 18 * 8 {
                    continue;
                }
                let mut bytes = Vec::with_capacity(out.len() / 8);
                for ch in out.chunks(8) {
                    let mut v = 0u8;
                    for (i, b) in ch.iter().enumerate() {
                        if *b {
                            v |= 1 << i;
                        }
                    }
                    bytes.push(v);
                }
                let n = bytes.len();
                let fcs = super::crc_x25(&bytes[..n - 2]);
                if bytes[n - 2] == (fcs & 0xFF) as u8 && bytes[n - 1] == (fcs >> 8) as u8 {
                    return Some(bytes[..n - 2].to_vec());
                }
            }
        }
        None
    }

    pub fn decode_call(bytes: &[u8]) -> String {
        let base: String = bytes[..6]
            .iter()
            .map(|b| (b >> 1) as char)
            .collect::<String>()
            .trim()
            .to_string();
        let ssid = (bytes[6] >> 1) & 0x0F;
        if ssid > 0 {
            format!("{base}-{ssid}")
        } else {
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_vector() {
        // standard CRC-16/X-25 check value
        assert_eq!(crc_x25(b"123456789"), 0x906E);
    }

    #[test]
    fn frame_roundtrip_through_hdlc() {
        let f = frame_bytes(
            "N0DECK-9",
            "APDECK",
            &["WIDE1-1"],
            "!4447.40N/02027.60E>test",
        );
        let bits = hdlc_bits(&f, 4, 2);
        // no run of six 1s outside flags
        let inner = &bits[4 * 8..bits.len() - 2 * 8];
        let mut run = 0;
        for b in inner {
            run = if *b { run + 1 } else { 0 };
            assert!(run < 6, "bit stuffing failed");
        }
        let frame = hdlc_rx::extract_frame(&bits).expect("frame recovered");
        assert_eq!(hdlc_rx::decode_call(&frame[0..7]), "APDECK");
        assert_eq!(hdlc_rx::decode_call(&frame[7..14]), "N0DECK-9");
        assert_eq!(frame[21], 0x03);
        assert_eq!(frame[22], 0xF0);
        assert_eq!(&frame[23..], b"!4447.40N/02027.60E>test");
    }

    #[test]
    fn nrzi_zero_toggles() {
        let lv = nrzi(&[true, false, false, true]);
        assert_eq!(lv, vec![true, false, true, true]);
        let back = hdlc_rx::nrzi_decode(&[true, lv[0], lv[1], lv[2], lv[3]][..]);
        assert_eq!(back, vec![true, false, false, true]);
    }
}
