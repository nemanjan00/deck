//! Embedded world map: Natural Earth 110m coastlines + country borders
//! (public domain), packed as little-endian f32 (lon, lat) pairs with
//! NaN,NaN as segment breaks. ~68 KB — offline geography for the radar.

use std::sync::OnceLock;

static DATA: &[u8] = include_bytes!("world.bin");
static SEGS: OnceLock<Vec<Vec<(f64, f64)>>> = OnceLock::new();

/// All polyline segments as (lat, lon) points.
pub fn segments() -> &'static [Vec<(f64, f64)>] {
    SEGS.get_or_init(|| {
        let mut out = Vec::new();
        let mut cur: Vec<(f64, f64)> = Vec::new();
        for ch in DATA.chunks_exact(8) {
            let lon = f32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]);
            let lat = f32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
            if lon.is_nan() || lat.is_nan() {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            } else {
                cur.push((f64::from(lat), f64::from(lon)));
            }
        }
        if cur.len() > 1 {
            out.push(cur);
        }
        out
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn world_data_parses() {
        let segs = super::segments();
        assert!(segs.len() > 300, "got {} segments", segs.len());
        let pts: usize = segs.iter().map(|s| s.len()).sum();
        assert!(pts > 5_000);
        // sanity: coordinates are on Earth
        for s in segs.iter().take(50) {
            for (lat, lon) in s {
                assert!((-90.0..=90.0).contains(lat));
                assert!((-180.0..=180.0).contains(lon));
            }
        }
    }
}
