//! Simulated ADS-B traffic as SBS/BaseStation lines (what dump1090 serves
//! on port 30003 — and what rtl1090 serves, for that matter).

use crate::dsp::Rng;

pub struct Plane {
    icao: u32,
    callsign: String,
    lat: f64,
    lon: f64,
    alt: f64,
    gs: f64,
    trk: f64,
    vs: f64,
    squawk: u16,
}

pub struct Fleet {
    rng: Rng,
    pub planes: Vec<Plane>,
    step_n: u64,
}

const AIRLINES: &[&str] = &["ASL", "JAT", "DLH", "WZZ", "THY", "AFR", "RYR", "AUA"];

impl Fleet {
    pub fn new(seed: u64, n: usize, lat0: f64, lon0: f64) -> Self {
        let mut rng = Rng::new(seed ^ 0xADB);
        let planes = (0..n)
            .map(|_| {
                let airline = *rng.pick(AIRLINES);
                Plane {
                    icao: (rng.next_u64() & 0xFF_FFFF) as u32,
                    callsign: format!("{airline}{}", rng.range_u32(100, 9999)),
                    lat: lat0 + rng.range_f64(-1.2, 1.2),
                    lon: lon0 + rng.range_f64(-1.6, 1.6),
                    alt: rng.range_f64(4_000.0, 39_000.0),
                    gs: rng.range_f64(160.0, 480.0),
                    trk: rng.range_f64(0.0, 360.0),
                    vs: *rng.pick(&[-1200.0, -600.0, 0.0, 0.0, 0.0, 800.0, 1600.0]),
                    squawk: 1000 + rng.range_u32(0, 6777) as u16,
                }
            })
            .collect();
        Self {
            rng,
            planes,
            step_n: 0,
        }
    }

    /// Advance the fleet by `dt` seconds and emit one round of SBS lines.
    pub fn step(&mut self, dt: f64) -> Vec<String> {
        self.step_n += 1;
        let now = chrono::Local::now();
        let d = now.format("%Y/%m/%d");
        let t = now.format("%H:%M:%S%.3f");
        let mut out = Vec::with_capacity(self.planes.len() * 2 + 1);
        for p in &mut self.planes {
            p.trk = (p.trk + self.rng.range_f64(-1.5, 1.5) * dt).rem_euclid(360.0);
            if self.rng.f64() < 0.01 * dt {
                p.vs = *self.rng.pick(&[-1600.0, -800.0, 0.0, 0.0, 900.0, 1500.0]);
            }
            p.alt = (p.alt + p.vs / 60.0 * dt).clamp(1_500.0, 41_000.0);
            let nm = p.gs * dt / 3600.0;
            let rad = p.trk.to_radians();
            p.lat += nm / 60.0 * rad.cos();
            p.lon += nm / 60.0 * rad.sin() / p.lat.to_radians().cos().max(0.2);

            let hex = format!("{:06X}", p.icao);
            out.push(format!(
                "MSG,3,1,1,{hex},1,{d},{t},{d},{t},,{alt},,,{lat:.5},{lon:.5},,,0,0,0,0",
                alt = p.alt.round() as i64,
                lat = p.lat,
                lon = p.lon,
            ));
            out.push(format!(
                "MSG,4,1,1,{hex},1,{d},{t},{d},{t},,,{gs:.1},{trk:.1},,,{vs},,,,,",
                gs = p.gs,
                trk = p.trk,
                vs = p.vs.round() as i64,
            ));
            if self.step_n % 10 == 1 {
                out.push(format!(
                    "MSG,1,1,1,{hex},1,{d},{t},{d},{t},{cs:<8},,,,,,,,,,,",
                    cs = p.callsign,
                ));
            }
            if self.step_n % 17 == 3 {
                out.push(format!(
                    "MSG,6,1,1,{hex},1,{d},{t},{d},{t},,,,,,,,{sq:04},0,0,0,0",
                    sq = p.squawk,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::sbs::AircraftStore;
    use std::time::Instant;

    #[test]
    fn fleet_feeds_the_real_parser() {
        let mut fleet = Fleet::new(7, 5, 44.8, 20.3);
        let mut store = AircraftStore::new();
        let t0 = Instant::now();
        for _ in 0..12 {
            for line in fleet.step(1.0) {
                assert!(store.push_line(&line, t0), "parser rejected: {line}");
            }
        }
        assert_eq!(store.len(), 5);
        let rows = store.rows(t0);
        // callsigns arrive on the MSG,1 cadence; positions/speeds every step
        assert!(rows.iter().all(|(a, _)| a.lat.is_some() && a.gs.is_some()));
        assert!(rows.iter().any(|(a, _)| !a.callsign.is_empty()));
        assert!(rows.iter().all(|(a, _)| a.msgs >= 20));
    }

    #[test]
    fn deterministic_with_seed() {
        let a: Vec<String> = Fleet::new(42, 3, 44.8, 20.3).step(1.0);
        let b: Vec<String> = Fleet::new(42, 3, 44.8, 20.3).step(1.0);
        // timestamps differ; strip them before comparing
        let strip = |s: &String| {
            s.split(',')
                .enumerate()
                .filter(|(i, _)| ![6, 7, 8, 9].contains(i))
                .map(|(_, f)| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(a.iter().map(strip).collect::<Vec<_>>(), b.iter().map(strip).collect::<Vec<_>>());
    }
}
