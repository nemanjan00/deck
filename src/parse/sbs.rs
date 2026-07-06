//! SBS/BaseStation (port 30003) CSV → live aircraft table.
//! Field layout (0-based): 0 "MSG", 1 tx type, 4 hexident, 10 callsign,
//! 11 altitude, 12 ground speed, 13 track, 14 lat, 15 lon, 16 vrate,
//! 17 squawk.

use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct Aircraft {
    pub icao: String,
    pub callsign: String,
    pub alt: Option<i32>,
    pub gs: Option<f32>,
    pub trk: Option<f32>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub vr: Option<i32>,
    pub squawk: String,
    pub msgs: u32,
    /// recent positions (lat, lon), oldest first — the radar trail
    pub trail: Vec<(f64, f64)>,
}

pub struct AircraftStore {
    map: HashMap<String, (Aircraft, Instant)>,
    pub total_msgs: u64,
}

impl AircraftStore {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            total_msgs: 0,
        }
    }

    /// Returns true when the line was a valid SBS message.
    pub fn push_line(&mut self, line: &str, now: Instant) -> bool {
        let f: Vec<&str> = line.split(',').collect();
        if f.first() != Some(&"MSG") || f.len() < 11 {
            return false;
        }
        let icao = f.get(4).unwrap_or(&"").trim().to_uppercase();
        if icao.is_empty() {
            return false;
        }
        self.total_msgs += 1;
        let entry = self.map.entry(icao.clone()).or_insert_with(|| {
            (
                Aircraft {
                    icao,
                    ..Default::default()
                },
                now,
            )
        });
        let (ac, last) = entry;
        *last = now;
        ac.msgs += 1;

        let get = |i: usize| f.get(i).map(|s| s.trim()).filter(|s| !s.is_empty());
        if let Some(cs) = get(10) {
            ac.callsign = cs.to_string();
        }
        if let Some(v) = get(11).and_then(|s| s.parse::<f64>().ok()) {
            ac.alt = Some(v as i32);
        }
        if let Some(v) = get(12).and_then(|s| s.parse::<f32>().ok()) {
            ac.gs = Some(v);
        }
        if let Some(v) = get(13).and_then(|s| s.parse::<f32>().ok()) {
            ac.trk = Some(v);
        }
        if let (Some(la), Some(lo)) = (
            get(14).and_then(|s| s.parse::<f64>().ok()),
            get(15).and_then(|s| s.parse::<f64>().ok()),
        ) {
            ac.lat = Some(la);
            ac.lon = Some(lo);
            if ac
                .trail
                .last()
                .map(|(a, b)| (a - la).abs() > 1e-5 || (b - lo).abs() > 1e-5)
                .unwrap_or(true)
            {
                ac.trail.push((la, lo));
                if ac.trail.len() > 24 {
                    ac.trail.remove(0);
                }
            }
        }
        if let Some(v) = get(16).and_then(|s| s.parse::<f64>().ok()) {
            ac.vr = Some(v as i32);
        }
        if let Some(sq) = get(17) {
            ac.squawk = sq.to_string();
        }
        true
    }

    pub fn purge(&mut self, now: Instant, max_age_s: u64) {
        self.map
            .retain(|_, (_, last)| now.duration_since(*last).as_secs() <= max_age_s);
    }

    /// Freshest first; ties broken by message count.
    pub fn rows(&self, now: Instant) -> Vec<(Aircraft, f32)> {
        let mut v: Vec<(Aircraft, f32)> = self
            .map
            .values()
            .map(|(ac, last)| (ac.clone(), now.duration_since(*last).as_secs_f32()))
            .collect();
        v.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.0.msgs.cmp(&a.0.msgs))
        });
        v
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[allow(dead_code)] // len()'s conventional companion; used in tests
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for AircraftStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn accumulates_fields_across_messages() {
        let mut st = AircraftStore::new();
        let t0 = Instant::now();
        assert!(st.push_line(
            "MSG,1,1,1,4CA9E2,1,2026/07/06,14:03:22.000,2026/07/06,14:03:22.000,JAT204  ,,,,,,,,,,,0",
            t0
        ));
        assert!(st.push_line(
            "MSG,3,1,1,4CA9E2,1,2026/07/06,14:03:23.000,2026/07/06,14:03:23.000,,37000,,,44.8125,20.3110,,,0,0,0,0",
            t0
        ));
        assert!(st.push_line(
            "MSG,4,1,1,4CA9E2,1,2026/07/06,14:03:24.000,2026/07/06,14:03:24.000,,,412.3,178.0,,,-640,,,,,",
            t0
        ));
        assert_eq!(st.len(), 1);
        let rows = st.rows(t0);
        let ac = &rows[0].0;
        assert_eq!(ac.callsign, "JAT204");
        assert_eq!(ac.alt, Some(37000));
        assert_eq!(ac.gs, Some(412.3));
        assert_eq!(ac.trk, Some(178.0));
        assert_eq!(ac.lat, Some(44.8125));
        assert_eq!(ac.vr, Some(-640));
        assert_eq!(ac.msgs, 3);
    }

    #[test]
    fn rejects_garbage_and_purges() {
        let mut st = AircraftStore::new();
        let t0 = Instant::now();
        assert!(!st.push_line("not,a,message", t0));
        assert!(!st.push_line("", t0));
        assert!(st.push_line(
            "MSG,3,1,1,ABCDEF,1,2026/07/06,14:00:00.000,2026/07/06,14:00:00.000,,1000,,,1.0,2.0,,,,,,",
            t0
        ));
        st.purge(t0 + Duration::from_secs(120), 60);
        assert!(st.is_empty());
    }
}
