//! ITA2 (Baudot) encoder for RTTY: 5-bit codes with LTRS/FIGS shifts,
//! framed as 1 start bit (space) + 5 data bits (LSB first) + 2 stop bits.

const LTRS: &str = "\0E\nA SIU\rDRJNFCKTZLWHYPQOBG\x0EMXV\x0F";
const FIGS: &str = "\x003\n- \x0787\r\x054',!:(5\")2#6019?&\x0E./;\x0F";
pub const SHIFT_LTRS: u8 = 0x1F;
pub const SHIFT_FIGS: u8 = 0x1B;

fn lookup(c: char) -> Option<(u8, bool)> {
    let c = c.to_ascii_uppercase();
    if let Some(i) = LTRS.find(c) {
        if i != 0x0E && i != 0x0F {
            return Some((i as u8, false));
        }
    }
    if let Some(i) = FIGS.find(c) {
        if i != 0x0E && i != 0x0F {
            return Some((i as u8, true));
        }
    }
    None
}

/// Text → framed air bits (true = mark). Idles a few mark bits up front and
/// re-asserts the shift state at the start.
pub fn encode(text: &str) -> Vec<bool> {
    let mut bits: Vec<bool> = Vec::new();
    let mut push_code = |bits: &mut Vec<bool>, code: u8| {
        bits.push(false); // start (space)
        for i in 0..5 {
            bits.push(code >> i & 1 == 1);
        }
        bits.push(true); // stop (mark)
        bits.push(true);
    };
    for _ in 0..8 {
        bits.push(true); // idle mark
    }
    let mut figs = false;
    push_code(&mut bits, SHIFT_LTRS);
    for c in text.chars() {
        let Some((code, wants_figs)) = lookup(c) else {
            continue;
        };
        if wants_figs != figs {
            figs = wants_figs;
            push_code(&mut bits, if figs { SHIFT_FIGS } else { SHIFT_LTRS });
        }
        push_code(&mut bits, code);
    }
    bits
}

/// Test-side decoder (also used by unit tests elsewhere).
pub fn decode(bits: &[bool]) -> String {
    let mut out = String::new();
    let mut figs = false;
    let mut i = 0;
    while i + 7 <= bits.len() {
        if bits[i] {
            i += 1; // idle mark
            continue;
        }
        let mut code = 0u8;
        for k in 0..5 {
            if bits[i + 1 + k] {
                code |= 1 << k;
            }
        }
        // require at least one stop bit
        if i + 6 < bits.len() && !bits[i + 6] {
            i += 1;
            continue;
        }
        match code {
            SHIFT_LTRS => figs = false,
            SHIFT_FIGS => figs = true,
            _ => {
                let table = if figs { FIGS } else { LTRS };
                if let Some(c) = table.chars().nth(code as usize) {
                    if c != '\0' {
                        out.push(c);
                    }
                }
            }
        }
        i += 8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_shifts() {
        let text = "RYRYRY DE DECK 599 QTH BEOGRAD";
        let bits = encode(text);
        assert_eq!(decode(&bits), text);
    }

    #[test]
    fn digits_need_figs() {
        let bits = encode("A1B2");
        assert_eq!(decode(&bits), "A1B2");
    }
}
