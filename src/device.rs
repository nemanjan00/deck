//! SDR detection. Pure sysfs — no libusb dependency, works unprivileged.
//! A "Simulator" device is always present so the whole UI runs offline.

use std::fs;
use std::ops::RangeInclusive;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SdrKind {
    RtlSdr,
    AirspyHf,
    Sim,
}

impl SdrKind {
    /// stable key used for pipeline template lookup
    pub fn key(self) -> &'static str {
        match self {
            SdrKind::RtlSdr => "rtlsdr",
            SdrKind::AirspyHf => "airspyhf",
            SdrKind::Sim => "sim",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SdrKind::RtlSdr => "RTL-SDR",
            SdrKind::AirspyHf => "Airspy HF+",
            SdrKind::Sim => "Simulator",
        }
    }

    /// Tunable ranges in Hz.
    pub fn ranges(self) -> &'static [RangeInclusive<u64>] {
        match self {
            SdrKind::RtlSdr => &[24_000_000..=1_766_000_000],
            SdrKind::AirspyHf => &[9_000..=31_000_000, 60_000_000..=260_000_000],
            SdrKind::Sim => &[0..=9_999_999_999],
        }
    }
}

/// USB VID:PID → kind table. Extend here to teach deck new radios.
const USB_IDS: &[(u16, u16, SdrKind)] = &[
    (0x0bda, 0x2832, SdrKind::RtlSdr),
    (0x0bda, 0x2838, SdrKind::RtlSdr),
    (0x03eb, 0x800c, SdrKind::AirspyHf), // Airspy HF+ / HF+ Discovery
];

#[derive(Clone, Debug)]
pub struct SdrDevice {
    pub kind: SdrKind,
    /// index among devices of the same kind (what rtl_* -d expects)
    pub index: u32,
    pub product: String,
    pub serial: Option<String>,
    pub usb_path: String,
}

impl SdrDevice {
    pub fn sim() -> Self {
        Self {
            kind: SdrKind::Sim,
            index: 0,
            product: "Built-in signal simulator".into(),
            serial: None,
            usb_path: String::new(),
        }
    }

    /// "rtlsdr:0", "sim" — stable id persisted in state.toml
    pub fn stable_id(&self) -> String {
        match self.kind {
            SdrKind::Sim => "sim".into(),
            k => format!("{}:{}", k.key(), self.index),
        }
    }

    pub fn freq_ok(&self, hz: u64) -> bool {
        self.kind.ranges().iter().any(|r| r.contains(&hz))
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Scan a sysfs usb devices dir (normally /sys/bus/usb/devices).
pub fn detect_in(sysfs: &Path) -> Vec<SdrDevice> {
    let mut found: Vec<(String, SdrKind, String, Option<String>)> = Vec::new();
    if let Ok(rd) = fs::read_dir(sysfs) {
        for e in rd.flatten() {
            let p = e.path();
            let (Some(vid), Some(pid)) = (
                read_trim(&p.join("idVendor")),
                read_trim(&p.join("idProduct")),
            ) else {
                continue;
            };
            let (Ok(vid), Ok(pid)) = (u16::from_str_radix(&vid, 16), u16::from_str_radix(&pid, 16))
            else {
                continue;
            };
            if let Some((_, _, kind)) = USB_IDS.iter().find(|(v, p, _)| *v == vid && *p == pid) {
                let product = read_trim(&p.join("product")).unwrap_or_else(|| kind.label().into());
                let serial = read_trim(&p.join("serial"));
                let path = e.file_name().to_string_lossy().to_string();
                found.push((path, *kind, product, serial));
            }
        }
    }
    // stable ordering ≈ librtlsdr enumeration order (bus/port order)
    found.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Vec::new();
    let mut counts: std::collections::HashMap<&'static str, u32> = Default::default();
    for (path, kind, product, serial) in found {
        let idx = counts.entry(kind.key()).or_insert(0);
        out.push(SdrDevice {
            kind,
            index: *idx,
            product,
            serial,
            usb_path: path,
        });
        *idx += 1;
    }
    out.push(SdrDevice::sim());
    out
}

pub fn detect() -> Vec<SdrDevice> {
    detect_in(Path::new("/sys/bus/usb/devices"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_dev(dir: &Path, name: &str, vid: &str, pid: &str, product: &str, serial: &str) {
        let d = dir.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("idVendor"), vid).unwrap();
        fs::write(d.join("idProduct"), pid).unwrap();
        fs::write(d.join("product"), product).unwrap();
        fs::write(d.join("serial"), serial).unwrap();
    }

    #[test]
    fn detects_and_indexes() {
        let tmp = std::env::temp_dir().join(format!("deck-test-sysfs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fake_dev(&tmp, "1-2", "0bda", "2838", "RTL2838UHIDIR", "00000001");
        fake_dev(&tmp, "1-3", "03eb", "800c", "AIRSPY HF+", "AH123");
        fake_dev(&tmp, "1-4", "0bda", "2838", "RTL2838UHIDIR", "00000002");
        fake_dev(&tmp, "1-5", "dead", "beef", "not an sdr", "x");

        let devs = detect_in(&tmp);
        // 2 rtl + 1 airspy + sim
        assert_eq!(devs.len(), 4);
        assert_eq!(devs[0].kind, SdrKind::RtlSdr);
        assert_eq!(devs[0].index, 0);
        assert_eq!(devs[2].index, 1); // second rtl
        assert_eq!(devs[1].kind, SdrKind::AirspyHf);
        assert_eq!(devs[3].kind, SdrKind::Sim);
        assert_eq!(devs[0].stable_id(), "rtlsdr:0");
        assert_eq!(devs[3].stable_id(), "sim");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ranges() {
        let rtl = SdrDevice {
            kind: SdrKind::RtlSdr,
            index: 0,
            product: String::new(),
            serial: None,
            usb_path: String::new(),
        };
        assert!(rtl.freq_ok(1_090_000_000));
        assert!(!rtl.freq_ok(7_100_000)); // HF: not without direct sampling
        let hf = SdrDevice {
            kind: SdrKind::AirspyHf,
            ..rtl.clone()
        };
        assert!(hf.freq_ok(7_100_000));
        assert!(hf.freq_ok(144_800_000));
        assert!(!hf.freq_ok(1_090_000_000));
    }
}
