//! POCSAG encoder: BCH(31,21) + even parity codewords, batch framing,
//! alpha & numeric messages. Spec-correct enough that multimon-ng decodes it.

pub const SYNC: u32 = 0x7CD2_15D8;
pub const IDLE: u32 = 0x7A89_C197;
const GEN: u32 = 0x769; // x^10+x^9+x^8+x^6+x^5+x^3+1

/// 21 information bits → 32-bit codeword (BCH check bits + even parity).
pub fn codeword(data21: u32) -> u32 {
    debug_assert!(data21 < (1 << 21));
    let mut rem = data21 << 10; // degree-30 dividend, data in bits 30..10
    for i in (10..31).rev() {
        if rem & (1 << i) != 0 {
            rem ^= GEN << (i - 10);
        }
    }
    let full31 = (data21 << 10) | (rem & 0x3FF);
    let word = full31 << 1;
    word | (word.count_ones() & 1)
}

/// Zero iff `w` is a valid POCSAG codeword (BCH syndrome + parity).
pub fn syndrome(w: u32) -> u32 {
    let mut rem = w >> 1;
    for i in (10..31).rev() {
        if rem & (1 << i) != 0 {
            rem ^= GEN << (i - 10);
        }
    }
    (rem & 0x3FF) | ((w.count_ones() & 1) << 10)
}

fn address_codeword(addr: u32, func: u8) -> u32 {
    // data21 = [0][addr>>3 (18b)][func (2b)]
    codeword(((addr >> 3) & 0x3_FFFF) << 2 | u32::from(func & 3))
}

fn message_codeword(bits20: u32) -> u32 {
    codeword(1 << 20 | (bits20 & 0xF_FFFF))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Content<'a> {
    Alpha(&'a str),
    Numeric(&'a str),
    ToneOnly,
}

/// Message payload → 20-bit chunks (MSB-first fill; chars LSB-first).
fn payload_chunks(content: Content) -> Vec<u32> {
    let mut bits: Vec<bool> = Vec::new();
    match content {
        Content::Alpha(text) => {
            for c in text.chars() {
                let b = (c as u32) & 0x7F;
                for i in 0..7 {
                    bits.push(b >> i & 1 == 1); // LSB first
                }
            }
        }
        Content::Numeric(digits) => {
            for c in digits.chars() {
                let nib: u32 = match c {
                    '0'..='9' => c as u32 - '0' as u32,
                    'U' | 'u' => 0xB,
                    ' ' => 0xC,
                    '-' => 0xD,
                    ')' => 0xE,
                    '(' => 0xF,
                    _ => continue,
                };
                for i in 0..4 {
                    bits.push(nib >> i & 1 == 1);
                }
            }
        }
        Content::ToneOnly => return Vec::new(),
    }
    bits.chunks(20)
        .map(|chunk| {
            let mut v = 0u32;
            for (i, b) in chunk.iter().enumerate() {
                if *b {
                    v |= 1 << (19 - i); // first bit lands in the MSB
                }
            }
            v
        })
        .collect()
}

/// Complete transmission: preamble + batches, as air bits (MSB first).
pub fn transmission_bits(addr: u32, func: u8, content: Content) -> Vec<bool> {
    let mut words: Vec<u32> = Vec::new();
    let frame = (addr & 7) as usize;
    let mut slot = 0usize;
    // idle up to the address frame
    while slot < frame * 2 {
        words.push(IDLE);
        slot += 1;
    }
    words.push(address_codeword(addr, func));
    slot += 1;
    for chunk in payload_chunks(content) {
        words.push(message_codeword(chunk));
        slot += 1;
    }
    // pad the final batch
    while slot % 16 != 0 {
        words.push(IDLE);
        slot += 1;
    }

    let mut bits: Vec<bool> = Vec::with_capacity(576 + words.len() * 34);
    for i in 0..576 {
        bits.push(i % 2 == 0); // 1010… preamble
    }
    for (i, w) in words.iter().enumerate() {
        if i % 16 == 0 {
            push_word(&mut bits, SYNC);
        }
        push_word(&mut bits, *w);
    }
    bits
}

fn push_word(bits: &mut Vec<bool>, w: u32) {
    for i in (0..32).rev() {
        bits.push(w >> i & 1 == 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_word_is_self_consistent() {
        // the spec's idle codeword must be exactly what our encoder produces
        assert_eq!(codeword(IDLE >> 11), IDLE);
        assert_eq!(syndrome(IDLE), 0);
        assert_eq!(syndrome(SYNC), 0, "sync word is also a valid codeword");
    }

    #[test]
    fn random_codewords_have_zero_syndrome() {
        let mut rng = crate::dsp::Rng::new(99);
        for _ in 0..500 {
            let data = (rng.next_u64() & 0x1F_FFFF) as u32;
            let w = codeword(data);
            assert_eq!(syndrome(w), 0);
            // and a single flipped bit is detected
            let bad = w ^ (1 << (rng.next_u64() % 32));
            assert_ne!(syndrome(bad), 0);
        }
    }

    #[test]
    fn transmission_structure() {
        let bits = transmission_bits(1_234_567, 0, Content::Alpha("HI"));
        assert!(bits[..576].iter().enumerate().all(|(i, b)| *b == (i % 2 == 0)));
        let word_at = |start: usize| -> u32 {
            bits[start..start + 32]
                .iter()
                .fold(0, |acc, b| acc << 1 | u32::from(*b))
        };
        assert_eq!(word_at(576), SYNC);
        // frame = addr & 7 = 7 → address codeword sits in slot 14
        let addr_cw = word_at(576 + 32 + 14 * 32);
        assert_eq!(addr_cw >> 31, 0, "address codeword");
        assert_eq!(syndrome(addr_cw), 0);
        let recovered_addr = ((addr_cw >> 13) & 0x3_FFFF) << 3 | 7;
        assert_eq!(recovered_addr, 1_234_567);
        // message codeword follows immediately (slot 15)
        let msg_cw = word_at(576 + 32 + 15 * 32);
        assert_eq!(msg_cw >> 31, 1, "message codeword");
        assert_eq!(syndrome(msg_cw), 0);
    }

    #[test]
    fn alpha_bits_decode_back() {
        // decode "HI" back out of the message codewords
        let chunks = payload_chunks(Content::Alpha("HI"));
        let mut bits: Vec<bool> = Vec::new();
        for c in &chunks {
            for i in (0..20).rev() {
                bits.push(c >> i & 1 == 1);
            }
        }
        let mut chars = Vec::new();
        for ch in bits.chunks(7) {
            if ch.len() < 7 {
                break;
            }
            let mut v = 0u8;
            for (i, b) in ch.iter().enumerate() {
                if *b {
                    v |= 1 << i;
                }
            }
            chars.push(v);
        }
        assert_eq!(&chars[..2], b"HI");
    }
}
