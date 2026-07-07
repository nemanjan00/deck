//! The mode registry. Adding a new mode is: one `ModeId` variant, one entry
//! in `MODES` (demod + decoder template), and optionally a parser arm in
//! `parse/` plus a sim profile. See docs/ADDING_MODES.md.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ModeId {
    // analog listening
    Nfm,
    Wfm,
    Am,
    Usb,
    Lsb,
    // digital voice (dsd-neo)
    Dmr,
    Ysf,
    Dstar,
    Nxdn,
    P25,
    M17,
    // data
    Pocsag,
    Aprs,
    Rtty,
    Adsb,
    Ais,
    // tools
    Scanner,
    Waterfall,
}

/// How the right-hand pane renders while this mode runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    Voice,
    Pager,
    Aprs,
    Adsb,
    /// AIS ships — same table/radar as ADS-B, marine labels
    Ais,
    TextFeed,
    Audio,
    Scanner,
    Waterfall,
}

/// Internal demodulator applied to the IQ stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Demod {
    Nfm,
    Wfm,
    Am,
    Usb,
    Lsb,
    /// no demod — waterfall/band-scope only
    Raw,
}

/// Plumbing class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PipeKind {
    /// deck reads IQ from the SDR, demodulates internally, and (optionally)
    /// pipes audio into a decoder's stdin. The default for everything.
    Iq(Demod),
    /// A fully external pipeline (ADS-B via dump1090, sim line feeds).
    Extern,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Analog,
    Voice,
    Data,
    Tools,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Analog => "ANALOG",
            Section::Voice => "DIGITAL VOICE",
            Section::Data => "DATA / BEACONS",
            Section::Tools => "TOOLS",
        }
    }
}

pub struct ModeDef {
    pub id: ModeId,
    /// stable key used in config files ("dmr", "pocsag", ...)
    pub key: &'static str,
    pub label: &'static str,
    pub section: Section,
    pub desc: &'static str,
    /// 2-cell pictogram + ascii fallback (spare metadata for text UIs;
    /// the GUI paints vector icons by `key`)
    #[allow(dead_code)]
    pub icon: &'static str,
    #[allow(dead_code)]
    pub icon_ascii: &'static str,
    pub view: ViewKind,
    pub pipe: PipeKind,
    /// stdin-audio decoder command (s16le mono at `decoder_rate`)
    pub decoder: Option<&'static str>,
    pub decoder_rate: u32,
    /// monitor demodulated audio on the speaker by default
    pub audio_out: bool,
    pub default_hz: u64,
    pub presets: &'static [(&'static str, u64)],
}

const MULTIMON_POCSAG: &str = "multimon-ng -t raw -a POCSAG512 -a POCSAG1200 -a POCSAG2400 -";
const MULTIMON_AFSK: &str = "multimon-ng -t raw -a AFSK1200 -";
const RTTY_DECODER: &str = "sox -t raw -r 22050 -e signed -b 16 -c 1 - -t wav - \
                            | minimodem --rx --quiet rtty --file /dev/stdin";

macro_rules! dsd {
    // dsd-neo reads raw PCM16LE mono on stdin; the rate MUST be given with
    // -s or symbol timing is wrong and nothing syncs. deck feeds 48 kHz.
    ($flag:literal) => {
        concat!("dsd-neo ", $flag, " -i - -s 48000 -o pulse")
    };
}

pub const MODES: &[ModeDef] = &[
    ModeDef {
        id: ModeId::Nfm,
        key: "nfm",
        label: "NFM",
        section: Section::Analog,
        desc: "Narrow FM voice — PMR446, ham repeaters, marine",
        icon: "◖◗",
        icon_ascii: "((",
        view: ViewKind::Audio,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: None,
        decoder_rate: 0,
        audio_out: true,
        default_hz: 145_500_000,
        presets: &[
            ("2m calling", 145_500_000),
            ("PMR446 ch1", 446_006_250),
            ("70cm calling", 433_500_000),
            ("Marine ch16", 156_800_000),
        ],
    },
    ModeDef {
        id: ModeId::Wfm,
        key: "wfm",
        label: "WFM",
        section: Section::Analog,
        desc: "Wideband FM — broadcast radio",
        icon: "♫ ",
        icon_ascii: "))",
        view: ViewKind::Audio,
        pipe: PipeKind::Iq(Demod::Wfm),
        decoder: None,
        decoder_rate: 0,
        audio_out: true,
        default_hz: 100_000_000,
        presets: &[("98.8", 98_800_000), ("100.0", 100_000_000)],
    },
    ModeDef {
        id: ModeId::Am,
        key: "am",
        label: "AM",
        section: Section::Analog,
        desc: "AM — airband, MW/SW broadcast",
        icon: "∿ ",
        icon_ascii: "~",
        view: ViewKind::Audio,
        pipe: PipeKind::Iq(Demod::Am),
        decoder: None,
        decoder_rate: 0,
        audio_out: true,
        default_hz: 121_500_000,
        presets: &[
            ("Air guard", 121_500_000),
            ("Airband", 118_105_000),
            ("SW 49m", 6_070_000),
        ],
    },
    ModeDef {
        id: ModeId::Usb,
        key: "usb",
        label: "USB",
        section: Section::Analog,
        desc: "Upper sideband — HF voice/digi, 20m/10m",
        icon: "⊓∿",
        icon_ascii: "us",
        view: ViewKind::Audio,
        pipe: PipeKind::Iq(Demod::Usb),
        decoder: None,
        decoder_rate: 0,
        audio_out: true,
        default_hz: 14_230_000,
        presets: &[("20m SSB", 14_230_000), ("10m SSB", 28_400_000)],
    },
    ModeDef {
        id: ModeId::Lsb,
        key: "lsb",
        label: "LSB",
        section: Section::Analog,
        desc: "Lower sideband — HF voice, 40m/80m",
        icon: "∿⊔",
        icon_ascii: "ls",
        view: ViewKind::Audio,
        pipe: PipeKind::Iq(Demod::Lsb),
        decoder: None,
        decoder_rate: 0,
        audio_out: true,
        default_hz: 7_100_000,
        presets: &[("40m SSB", 7_100_000), ("80m SSB", 3_700_000)],
    },
    ModeDef {
        id: ModeId::Dmr,
        key: "dmr",
        label: "DMR",
        section: Section::Voice,
        desc: "DMR digital voice — TG, slot, color code",
        icon: "◉◉",
        icon_ascii: "oo",
        view: ViewKind::Voice,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(dsd!("-fs")),
        decoder_rate: 48000,
        audio_out: false, // dsd-neo plays decoded voice itself
        default_hz: 433_450_000,
        presets: &[("UHF simplex", 433_450_000), ("Hotspot", 438_800_000)],
    },
    ModeDef {
        id: ModeId::Ysf,
        key: "ysf",
        label: "YSF",
        section: Section::Voice,
        desc: "Yaesu System Fusion C4FM",
        icon: "◈ ",
        icon_ascii: "<>",
        view: ViewKind::Voice,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(dsd!("-fy")),
        decoder_rate: 48000,
        audio_out: false,
        default_hz: 433_450_000,
        presets: &[("UHF simplex", 433_450_000), ("Hotspot", 434_000_000)],
    },
    ModeDef {
        id: ModeId::Dstar,
        key: "dstar",
        label: "D-STAR",
        section: Section::Voice,
        desc: "Icom D-STAR digital voice",
        icon: "✦ ",
        icon_ascii: "*",
        view: ViewKind::Voice,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(dsd!("-fd")),
        decoder_rate: 48000,
        audio_out: false,
        default_hz: 145_375_000,
        presets: &[("2m", 145_375_000), ("70cm", 439_562_500)],
    },
    ModeDef {
        id: ModeId::Nxdn,
        key: "nxdn",
        label: "NXDN",
        section: Section::Voice,
        desc: "NXDN48 digital voice (RAN, TG)",
        icon: "◧ ",
        icon_ascii: "[]",
        view: ViewKind::Voice,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(dsd!("-fi")),
        decoder_rate: 48000,
        audio_out: false,
        default_hz: 451_000_000,
        presets: &[("UHF", 451_000_000)],
    },
    ModeDef {
        id: ModeId::P25,
        key: "p25",
        label: "P25",
        section: Section::Voice,
        desc: "APCO P25 Phase 1 (NAC, TG)",
        icon: "◬ ",
        icon_ascii: "/\\",
        view: ViewKind::Voice,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(dsd!("-f1")),
        decoder_rate: 48000,
        audio_out: false,
        default_hz: 460_100_000,
        presets: &[("UHF", 460_100_000), ("800", 851_012_500)],
    },
    ModeDef {
        id: ModeId::M17,
        key: "m17",
        label: "M17",
        section: Section::Voice,
        desc: "M17 open digital voice",
        icon: "Ⓜ ",
        icon_ascii: "M",
        view: ViewKind::Voice,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(dsd!("-fz")),
        decoder_rate: 48000,
        audio_out: false,
        default_hz: 433_475_000,
        presets: &[("70cm", 433_475_000)],
    },
    ModeDef {
        id: ModeId::Pocsag,
        key: "pocsag",
        label: "POCSAG",
        section: Section::Data,
        desc: "Pager messages — 512/1200/2400 baud",
        icon: "▤ ",
        icon_ascii: "=",
        view: ViewKind::Pager,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(MULTIMON_POCSAG),
        decoder_rate: 22050,
        audio_out: false, // toggle monitor with 'a' if you like the chirps
        default_hz: 169_650_000,
        presets: &[
            ("P2000 NL", 169_650_000),
            ("e*Msg DE", 466_075_000),
            ("VHF", 148_562_500),
        ],
    },
    ModeDef {
        id: ModeId::Aprs,
        key: "aprs",
        label: "APRS",
        section: Section::Data,
        desc: "APRS AFSK1200 packets",
        icon: "⌖ ",
        icon_ascii: "@",
        view: ViewKind::Aprs,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: Some(MULTIMON_AFSK),
        decoder_rate: 22050,
        audio_out: false,
        default_hz: 144_800_000,
        presets: &[
            ("EU", 144_800_000),
            ("NA", 144_390_000),
            ("ISS", 145_825_000),
        ],
    },
    ModeDef {
        id: ModeId::Rtty,
        key: "rtty",
        label: "RTTY",
        section: Section::Data,
        desc: "Baudot RTTY 45.45Bd (via minimodem)",
        icon: "⌨ ",
        icon_ascii: "ry",
        view: ViewKind::TextFeed,
        pipe: PipeKind::Iq(Demod::Usb),
        decoder: Some(RTTY_DECODER),
        decoder_rate: 22050,
        audio_out: false,
        default_hz: 7_646_000,
        presets: &[("DWD 7.646", 7_646_000), ("DWD 10.100k8", 10_100_800)],
    },
    ModeDef {
        id: ModeId::Adsb,
        key: "adsb",
        label: "ADS-B",
        section: Section::Data,
        desc: "1090 MHz aircraft (dump1090/SBS)",
        icon: "✈ ",
        icon_ascii: "^",
        view: ViewKind::Adsb,
        pipe: PipeKind::Extern,
        decoder: None,
        decoder_rate: 0,
        audio_out: false,
        default_hz: 1_090_000_000,
        presets: &[("1090ES", 1_090_000_000)],
    },
    ModeDef {
        id: ModeId::Ais,
        key: "ais",
        label: "AIS",
        section: Section::Data,
        desc: "Ship positions — marine VHF (rtl_ais)",
        icon: "⛵",
        icon_ascii: "~^",
        view: ViewKind::Ais,
        pipe: PipeKind::Extern,
        decoder: None,
        decoder_rate: 0,
        audio_out: false,
        default_hz: 161_975_000,
        presets: &[("AIS ch A", 161_975_000), ("AIS ch B", 162_025_000)],
    },
    ModeDef {
        id: ModeId::Scanner,
        key: "scanner",
        label: "SCAN",
        section: Section::Tools,
        desc: "NFM channel scanner with lockout",
        icon: "⇄ ",
        icon_ascii: "<->",
        view: ViewKind::Scanner,
        pipe: PipeKind::Iq(Demod::Nfm),
        decoder: None,
        decoder_rate: 0,
        audio_out: true,
        default_hz: 446_006_250,
        presets: &[],
    },
    ModeDef {
        id: ModeId::Waterfall,
        key: "waterfall",
        label: "WFALL",
        section: Section::Tools,
        desc: "RF spectrum waterfall (raw IQ)",
        icon: "▓▒",
        icon_ascii: "#:",
        view: ViewKind::Waterfall,
        pipe: PipeKind::Iq(Demod::Raw),
        decoder: None,
        decoder_rate: 0,
        audio_out: false,
        default_hz: 433_920_000,
        presets: &[
            ("ISM 433", 433_920_000),
            ("2m", 145_000_000),
            ("Airband", 124_000_000),
        ],
    },
];

pub fn mode_def(id: ModeId) -> &'static ModeDef {
    MODES.iter().find(|m| m.id == id).expect("mode registered")
}

#[allow(dead_code)] // public lookup API, exercised by tests
pub fn mode_by_key(key: &str) -> Option<&'static ModeDef> {
    MODES.iter().find(|m| m.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_consistent() {
        for m in MODES {
            assert!(!m.key.is_empty());
            assert!(mode_def(m.id).key == m.key);
            assert_eq!(mode_by_key(m.key).unwrap().id, m.id);
            if m.decoder.is_some() {
                assert!(m.decoder_rate > 0, "{} needs decoder_rate", m.key);
            }
        }
    }
}
