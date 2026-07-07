//! AIS (marine AIVDM/NMEA) decoder: position reports (types 1/2/3/18) and
//! vessel names (5, 24A), with multi-fragment reassembly. Ships land in the
//! same store as aircraft — the radar map doesn't care what floats.

use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AisMsg {
    pub mmsi: u32,
    pub msg_type: u8,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// speed over ground, knots
    pub sog: Option<f32>,
    /// course over ground, degrees
    pub cog: Option<f32>,
    pub name: Option<String>,
}

fn checksum_ok(s: &str) -> bool {
    let Some(body) = s.strip_prefix('!') else {
        return false;
    };
    let Some((data, sum)) = body.rsplit_once('*') else {
        return false;
    };
    let want = u8::from_str_radix(sum.trim(), 16).unwrap_or(0xFF);
    let got = data.bytes().fold(0u8, |a, b| a ^ b);
    want == got
}

/// 6-bit de-armoring per ITU-R M.1371.
fn payload_bits(payload: &str) -> Vec<bool> {
    let mut bits = Vec::with_capacity(payload.len() * 6);
    for c in payload.bytes() {
        let mut v = c.wrapping_sub(48);
        if v > 40 {
            v -= 8;
        }
        for i in (0..6).rev() {
            bits.push((v >> i) & 1 == 1);
        }
    }
    bits
}

fn take_u(bits: &[bool], start: usize, len: usize) -> u32 {
    bits[start..(start + len).min(bits.len())]
        .iter()
        .fold(0u32, |a, b| (a << 1) | u32::from(*b))
}

fn take_i(bits: &[bool], start: usize, len: usize) -> i32 {
    let v = take_u(bits, start, len);
    // sign-extend
    if v & (1 << (len - 1)) != 0 {
        (v | (u32::MAX << len)) as i32
    } else {
        v as i32
    }
}

fn take_str(bits: &[bool], start: usize, len: usize) -> String {
    let mut out = String::new();
    let mut i = start;
    while i + 6 <= (start + len).min(bits.len()) {
        let c = take_u(bits, i, 6) as u8;
        let ch = if c < 32 { c + 64 } else { c }; // 6-bit ASCII
        if ch as char == '@' {
            break;
        }
        out.push(ch as char);
        i += 6;
    }
    out.trim().to_string()
}

fn decode_payload(bits: &[bool]) -> Option<AisMsg> {
    if bits.len() < 38 {
        return None;
    }
    let t = take_u(bits, 0, 6) as u8;
    let mmsi = take_u(bits, 8, 30);
    let mut m = AisMsg {
        mmsi,
        msg_type: t,
        ..Default::default()
    };
    let latlon = |m: &mut AisMsg, lon_at: usize, lat_at: usize| {
        let lon = take_i(bits, lon_at, 28);
        let lat = take_i(bits, lat_at, 27);
        if lon != 0x6791AC0 && lat != 0x3412140 {
            m.lon = Some(f64::from(lon) / 600_000.0);
            m.lat = Some(f64::from(lat) / 600_000.0);
        }
    };
    match t {
        1..=3 => {
            let sog = take_u(bits, 50, 10);
            if sog != 1023 {
                m.sog = Some(sog as f32 / 10.0);
            }
            latlon(&mut m, 61, 89);
            let cog = take_u(bits, 116, 12);
            if cog != 3600 {
                m.cog = Some(cog as f32 / 10.0);
            }
        }
        18 => {
            let sog = take_u(bits, 46, 10);
            if sog != 1023 {
                m.sog = Some(sog as f32 / 10.0);
            }
            latlon(&mut m, 57, 85);
            let cog = take_u(bits, 112, 12);
            if cog != 3600 {
                m.cog = Some(cog as f32 / 10.0);
            }
        }
        5 => {
            if bits.len() >= 232 {
                let name = take_str(bits, 112, 120);
                if !name.is_empty() {
                    m.name = Some(name);
                }
            }
        }
        24 if take_u(bits, 38, 2) == 0 && bits.len() >= 160 => {
            let name = take_str(bits, 40, 120);
            if !name.is_empty() {
                m.name = Some(name);
            }
        }
        _ => {}
    }
    Some(m)
}

/// Stateful AIVDM sentence parser (handles fragmenting).
#[derive(Default)]
pub struct AisParser {
    frags: HashMap<(String, String), Vec<Option<String>>>,
}

impl AisParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, line: &str) -> Option<AisMsg> {
        let line = line.trim();
        if !(line.starts_with("!AIVDM") || line.starts_with("!AIVDO")) || !checksum_ok(line) {
            return None;
        }
        let body = &line[1..line.rfind('*')?];
        let f: Vec<&str> = body.split(',').collect();
        if f.len() < 7 {
            return None;
        }
        let total: usize = f[1].parse().ok()?;
        let num: usize = f[2].parse().ok()?;
        let payload = f[5].to_string();
        if total <= 1 {
            return decode_payload(&payload_bits(&payload));
        }
        // reassemble
        let key = (f[3].to_string(), f[4].to_string());
        let slots = self
            .frags
            .entry(key.clone())
            .or_insert_with(|| vec![None; total]);
        if num >= 1 && num <= slots.len() {
            slots[num - 1] = Some(payload);
        }
        if slots.iter().all(|s| s.is_some()) {
            let joined: String = slots.iter().flatten().cloned().collect();
            self.frags.remove(&key);
            return decode_payload(&payload_bits(&joined));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_and_type() {
        // classic sample sentence (gpsd docs)
        let s = "!AIVDM,1,1,,B,177KQJ5000G?tO`K>RA1wUbN0TKH,0*5C";
        assert!(checksum_ok(s));
        let m = AisParser::new().push(s).unwrap();
        assert_eq!(m.msg_type, 1);
        assert_eq!(m.mmsi, 477553000);
        assert!(m.lat.is_some() && m.lon.is_some());
    }

    #[test]
    fn rejects_bad_checksum() {
        let s = "!AIVDM,1,1,,B,177KQJ5000G?tO`K>RA1wUbN0TKH,0*5D";
        assert!(AisParser::new().push(s).is_none());
    }

    #[test]
    fn roundtrip_with_sim_encoder() {
        let sentences = crate::sim::ais::encode_type1(244_123_456, 44.87, 13.85, 8.4, 231.0);
        let mut p = AisParser::new();
        let m = p.push(&sentences).expect("decodes");
        assert_eq!(m.mmsi, 244_123_456);
        assert!((m.lat.unwrap() - 44.87).abs() < 0.0002);
        assert!((m.lon.unwrap() - 13.85).abs() < 0.0002);
        assert!((m.sog.unwrap() - 8.4).abs() < 0.11);
        assert!((m.cog.unwrap() - 231.0).abs() < 0.11);

        let name = crate::sim::ais::encode_name24(244_123_456, "SEA DECK");
        let m2 = p.push(&name).expect("name decodes");
        assert_eq!(m2.name.as_deref(), Some("SEA DECK"));
    }
}
