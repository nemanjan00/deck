//! Config (~/.config/deck/config.toml) + persisted state (~/.local/state/deck).
//! Everything has built-in defaults; the config file only overrides.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub sdr: SdrCfg,
    pub ui: UiCfg,
    pub audio: AudioCfg,
    pub adsb: AdsbCfg,
    pub scanner: ScanCfg,
    pub sweep: SweepCfg,
    /// extra frequency presets per mode key, appended to built-ins
    pub presets: HashMap<String, Vec<Chan>>,
    /// decoder command overrides per mode key (stdin s16le audio decoders)
    pub decoders: HashMap<String, String>,
    /// pipeline overrides: [pipelines.<device>] iq_source = "...", <mode> = "..."
    pub pipelines: HashMap<String, HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct SdrCfg {
    pub ppm: i32,
    pub gain: f32,
    /// display calibration offset in dB, added to band level readouts
    /// (relative levels unless you calibrate against a known source)
    pub cal_db: f32,
}
impl Default for SdrCfg {
    fn default() -> Self {
        Self {
            ppm: 0,
            gain: 32.8,
            cal_db: 0.0,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct UiCfg {
    /// "dark" | "light"
    pub theme: String,
    /// "unicode" | "ascii"
    pub icons: String,
    pub splash: bool,
    /// enable mouse/touch input
    pub mouse: bool,
}
impl Default for UiCfg {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            icons: "unicode".into(),
            splash: true,
            mouse: true,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct AudioCfg {
    /// "auto" or a sink command template, e.g.
    /// "paplay --raw --rate={rate} --format=s16le --channels=1"
    pub sink: String,
    /// recordings directory ("" = XDG music dir /deck)
    pub record_dir: String,
}
impl Default for AudioCfg {
    fn default() -> Self {
        Self {
            sink: "auto".into(),
            record_dir: String::new(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct AdsbCfg {
    pub sbs_host: String,
    pub sbs_port: u16,
    /// home position for the radar map (0,0 = auto-center on traffic)
    pub lat: f64,
    pub lon: f64,
}
impl Default for AdsbCfg {
    fn default() -> Self {
        Self {
            sbs_host: "127.0.0.1".into(),
            sbs_port: 30003,
            lat: 0.0,
            lon: 0.0,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Chan {
    pub label: String,
    pub hz: u64,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct SweepCfg {
    /// search-between-limits range for the waterfall SWEEP function
    pub from: u64,
    pub to: u64,
    pub dwell_ms: u64,
}
impl Default for SweepCfg {
    fn default() -> Self {
        Self {
            from: 430_000_000,
            to: 440_000_000,
            dwell_ms: 1200,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct ScanCfg {
    /// ms to listen on a silent channel before hopping
    pub dwell_ms: u64,
    /// ms to keep holding after the signal disappears
    pub hold_ms: u64,
    pub channels: Vec<Chan>,
}
impl Default for ScanCfg {
    fn default() -> Self {
        let mut channels: Vec<Chan> = (0..8)
            .map(|i| Chan {
                label: format!("PMR446 ch{}", i + 1),
                hz: 446_006_250 + i * 12_500,
            })
            .collect();
        channels.push(Chan {
            label: "2m calling".into(),
            hz: 145_500_000,
        });
        channels.push(Chan {
            label: "70cm calling".into(),
            hz: 433_500_000,
        });
        Self {
            dwell_ms: 450,
            hold_ms: 2500,
            channels,
        }
    }
}

/// Session state persisted across runs (not user-edited).
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PersistState {
    pub theme: Option<String>,
    pub device: Option<String>,
    pub modes: HashMap<String, ModePersist>,
    pub lockouts: Vec<u64>,
    /// scanner priority channel (checked between hops)
    pub priority: Option<u64>,
    /// saved memory channels (KC908-style), shown starred in preset pickers
    pub memories: Vec<Memory>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Memory {
    pub label: String,
    pub hz: u64,
    /// mode key it was saved from
    pub mode: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ModePersist {
    pub freq: u64,
    pub gain: f32,
    pub squelch: f32,
    /// spectral noise reduction level (0 = off)
    pub nr: u8,
    /// noise blanker level (0 = off)
    pub nb: u8,
    /// LMS auto-notch enabled
    pub notch: bool,
    /// audio high-pass / low-pass cutoffs (0 = off)
    pub hp: u32,
    pub lp: u32,
    /// AM detector: 0 = envelope, 1 = synchronous (SAM)
    pub det: u8,
    /// CTCSS tone squelch in Hz (0 = off)
    pub tone: f32,
    /// SSB passband (IF) shift in Hz
    pub if_shift: i32,
    /// monitor audio toggle for decoder modes
    pub monitor: bool,
}

pub fn config_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("deck/config.toml")
}

pub fn state_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("deck/state.toml")
}

pub fn load_config(explicit: Option<&Path>) -> (Config, Option<String>) {
    let path = config_path(explicit);
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str::<Config>(&s) {
            Ok(c) => (c, None),
            Err(e) => (
                Config::default(),
                Some(format!("config error in {}: {e}", path.display())),
            ),
        },
        Err(_) => (Config::default(), None),
    }
}

pub fn load_state() -> PersistState {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(st: &PersistState) {
    let p = state_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = toml::to_string_pretty(st) {
        let _ = std::fs::write(p, s);
    }
}

/// Annotated default config, written by `deck config --write`.
pub fn default_config_toml() -> String {
    r##"# deck — configuration
# Everything here is optional; delete anything to fall back to defaults.

[sdr]
ppm = 0        # frequency correction, applied where the tool supports it
gain = 32.8    # default tuner gain in dB (adjust live with +/-)

[ui]
theme = "dark"    # "dark" (hacker phosphor) | "light"
icons = "unicode" # "unicode" | "ascii" (for spartan fonts)
splash = true
mouse = true      # tap/scroll support (touchscreen decks)

[audio]
# "auto" picks the first of: aplay, paplay. Override with a template:
# sink = "aplay -q -t raw -f S16_LE -r {rate} -c 1 -"
# sink = "paplay --raw --rate={rate} --format=s16le --channels=1"
# sink = "pw-cat -p --rate {rate} --channels 1 --format s16 -"
sink = "auto"

[adsb]
sbs_host = "127.0.0.1"
sbs_port = 30003
# deck reads BaseStation (SBS) from this TCP port, so ANY 1090 decoder works:
# dump1090, readsb — or rtl1090 under wine (it serves the same feed on 30003).

[scanner]
dwell_ms = 450   # listen time per silent channel
hold_ms = 2500   # linger after a signal drops
# [[scanner.channels]]
# label = "PMR446 ch1"
# hz = 446006250

# Extra presets per mode (appended to built-ins):
# [[presets.nfm]]
# label = "Local repeater"
# hz = 438725000

# ── Advanced plumbing ────────────────────────────────────────────────
# deck reads raw IQ from the SDR, demodulates internally, and pipes audio
# into decoders' stdin. Template placeholders:
#   {device} {freq_hz} {freq_mhz} {freq_khz} {center_hz} {center_mhz}
#   {gain} {gain_int} {ppm} {rate} {sbs_port} {profile}
#   {deck} = path to this binary
#
# Decoder overrides (they receive s16le mono audio on stdin):
# [decoders]
# pocsag = "multimon-ng -t raw -a POCSAG1200 -"
# dmr    = "dsd-neo -fs -i - -o pulse -N"
#
# IQ source / external pipeline overrides per device:
# [pipelines.rtlsdr]
# iq_source = "rtl_sdr -d {device} -f {center_hz} -g {gain} -p {ppm} -s 2400000 -"
# adsb      = "readsb --device {device} --gain {gain} --quiet --net --net-sbs-port {sbs_port}"
#
# [pipelines.airspyhf]
# # airspyhf_rx builds differ; adjust flags if yours needs them:
# iq_source = "airspyhf_rx -f {center_mhz} -a 768000 -r /dev/stdout"
"##
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let cfg: Config = toml::from_str(&default_config_toml()).unwrap();
        assert_eq!(cfg.adsb.sbs_port, 30003);
        assert_eq!(cfg.ui.theme, "dark");
        assert_eq!(cfg.scanner.channels.len(), 10);
    }

    #[test]
    fn partial_config_merges_defaults() {
        let cfg: Config = toml::from_str("[ui]\ntheme = \"light\"\n").unwrap();
        assert_eq!(cfg.ui.theme, "light");
        assert!(cfg.ui.splash);
        assert_eq!(cfg.sdr.gain, 32.8);
    }

    #[test]
    fn state_roundtrip() {
        let mut st = PersistState::default();
        st.modes.insert(
            "nfm".into(),
            ModePersist {
                freq: 145_500_000,
                gain: 20.0,
                squelch: 0.05,
                nr: 2,
                ..Default::default()
            },
        );
        let s = toml::to_string_pretty(&st).unwrap();
        let back: PersistState = toml::from_str(&s).unwrap();
        assert_eq!(back.modes["nfm"].freq, 145_500_000);
    }
}
