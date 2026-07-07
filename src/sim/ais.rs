//! AIS sentence encoder + a slow-moving simulated fleet (AIVDM lines).

use crate::dsp::Rng;

fn armor(bits: &[bool]) -> String {
    let mut out = String::new();
    for ch in bits.chunks(6) {
        let mut v = 0u8;
        for (i, b) in ch.iter().enumerate() {
            if *b {
                v |= 1 << (5 - i);
            }
        }
        v += 48;
        if v > 87 {
            v += 8;
        }
        out.push(v as char);
    }
    out
}

fn push_u(bits: &mut Vec<bool>, v: u32, len: usize) {
    for i in (0..len).rev() {
        bits.push((v >> i) & 1 == 1);
    }
}

fn sentence(payload: &str) -> String {
    let body = format!("AIVDM,1,1,,A,{payload},0");
    let sum = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("!{body}*{sum:02X}")
}

/// Type 1 position report.
pub fn encode_type1(mmsi: u32, lat: f64, lon: f64, sog_kn: f32, cog_deg: f32) -> String {
    let mut b = Vec::with_capacity(168);
    push_u(&mut b, 1, 6); // type
    push_u(&mut b, 0, 2); // repeat
    push_u(&mut b, mmsi, 30);
    push_u(&mut b, 0, 4); // nav status
    push_u(&mut b, 128, 8); // ROT not available
    push_u(&mut b, (sog_kn * 10.0).round() as u32, 10);
    push_u(&mut b, 0, 1); // accuracy
    push_u(
        &mut b,
        ((lon * 600_000.0).round() as i32) as u32 & 0x0FFF_FFFF,
        28,
    );
    push_u(
        &mut b,
        ((lat * 600_000.0).round() as i32) as u32 & 0x07FF_FFFF,
        27,
    );
    push_u(&mut b, (cog_deg * 10.0).round() as u32 % 3600, 12);
    push_u(&mut b, 511, 9); // heading n/a
    push_u(&mut b, 60, 6); // timestamp n/a
    push_u(&mut b, 0, 2); // maneuver
    push_u(&mut b, 0, 3); // spare
    push_u(&mut b, 0, 1); // RAIM
    push_u(&mut b, 0, 19); // radio status
    sentence(&armor(&b))
}

/// Type 24 part A (vessel name, single fragment).
pub fn encode_name24(mmsi: u32, name: &str) -> String {
    let mut b = Vec::with_capacity(160);
    push_u(&mut b, 24, 6);
    push_u(&mut b, 0, 2);
    push_u(&mut b, mmsi, 30);
    push_u(&mut b, 0, 2); // part A
    let up = name.to_ascii_uppercase();
    let mut chars: Vec<u8> = up.bytes().take(20).collect();
    while chars.len() < 20 {
        chars.push(b'@');
    }
    for c in chars {
        let six = if c == b'@' { 0 } else { (c as u32) & 0x3F };
        push_u(&mut b, six, 6);
    }
    sentence(&armor(&b))
}

pub struct Boat {
    mmsi: u32,
    name: String,
    lat: f64,
    lon: f64,
    sog: f32,
    cog: f32,
}

pub struct BoatFleet {
    rng: Rng,
    pub boats: Vec<Boat>,
    step_n: u64,
}

const NAMES: &[&str] = &[
    "SEA DECK",
    "JADRAN",
    "BURA",
    "GALEB",
    "MARCO POLO",
    "NEPTUN",
    "ORKA",
    "TIVAT TRADER",
];

impl BoatFleet {
    pub fn new(seed: u64, n: usize, lat0: f64, lon0: f64) -> Self {
        let mut rng = Rng::new(seed ^ 0xB0A7);
        let boats = (0..n)
            .map(|i| Boat {
                mmsi: 244_000_000 + rng.range_u32(0, 999_999),
                name: NAMES[i % NAMES.len()].to_string(),
                lat: lat0 + rng.range_f64(-0.4, 0.4),
                lon: lon0 + rng.range_f64(-0.6, 0.6),
                sog: rng.range_f64(0.0, 18.0) as f32,
                cog: rng.range_f64(0.0, 360.0) as f32,
            })
            .collect();
        Self {
            rng,
            boats,
            step_n: 0,
        }
    }

    pub fn step(&mut self, dt: f64) -> Vec<String> {
        self.step_n += 1;
        let mut out = Vec::with_capacity(self.boats.len() + 1);
        for b in &mut self.boats {
            b.cog =
                (f64::from(b.cog) + self.rng.range_f64(-2.0, 2.0) * dt).rem_euclid(360.0) as f32;
            let nm = f64::from(b.sog) * dt / 3600.0;
            let rad = f64::from(b.cog).to_radians();
            b.lat += nm / 60.0 * rad.cos();
            b.lon += nm / 60.0 * rad.sin() / b.lat.to_radians().cos().max(0.2);
            out.push(encode_type1(b.mmsi, b.lat, b.lon, b.sog, b.cog));
            if self.step_n % 15 == 1 {
                out.push(encode_name24(b.mmsi, &b.name));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fleet_feeds_the_parser() {
        let mut fleet = super::BoatFleet::new(3, 5, 44.8, 13.9);
        let mut p = crate::parse::ais::AisParser::new();
        let mut named = 0;
        for _ in 0..16 {
            for line in fleet.step(1.0) {
                let m = p.push(&line).expect("parses");
                assert!(m.mmsi >= 244_000_000);
                if m.name.is_some() {
                    named += 1;
                }
            }
        }
        assert!(named >= 5, "names delivered: {named}");
    }
}
