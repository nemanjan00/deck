//! dsd-neo / DSD-FME output: tolerant field extraction for the live call
//! card. DMR gets slot + color code + TG + RID; YSF/D-STAR/M17 get their
//! callsign fields; NXDN RAN; P25 NAC.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallFields {
    pub slot: Option<u8>,
    pub cc: Option<String>,
    pub tg: Option<String>,
    pub src: Option<String>,
    pub dst: Option<String>,
    pub kind: Option<String>,
    pub extra: Vec<(String, String)>,
}

impl CallFields {
    pub fn is_empty(&self) -> bool {
        self == &CallFields::default()
    }

    pub fn merge(&mut self, other: CallFields) {
        if other.slot.is_some() {
            self.slot = other.slot;
        }
        if other.cc.is_some() {
            self.cc = other.cc;
        }
        if other.tg.is_some() {
            self.tg = other.tg;
        }
        if other.src.is_some() {
            self.src = other.src;
        }
        if other.dst.is_some() {
            self.dst = other.dst;
        }
        if other.kind.is_some() {
            self.kind = other.kind;
        }
        for (k, v) in other.extra {
            if let Some(e) = self.extra.iter_mut().find(|(ek, _)| *ek == k) {
                e.1 = v;
            } else {
                self.extra.push((k, v));
            }
        }
    }
}

/// Value following any of `keys` in `line`: `TG: 91`, `TG=91`, `TG 91`.
/// Both sides of the key must be token boundaries ("DST" must not match
/// inside "DSTAR").
fn field_after<'a>(line: &'a str, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        let mut start = 0;
        while let Some(i) = line[start..].find(key) {
            let at = start + i;
            let ok_before = at == 0
                || !line[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric());
            let after_raw = &line[at + key.len()..];
            let ok_after = after_raw.starts_with([':', '=', ' ']);
            let after = after_raw.trim_start_matches([':', '=']).trim_start();
            if ok_before && ok_after && !after.is_empty() {
                let end = after.find(['|', ',', ';']).unwrap_or(after.len());
                let val = after[..end].split_whitespace().next().unwrap_or("");
                if !val.is_empty() && val != ":" && val != "=" {
                    return Some(val);
                }
            }
            start = at + key.len();
        }
    }
    None
}

pub fn parse_line(line: &str) -> CallFields {
    let mut f = CallFields::default();
    let up = line.to_ascii_uppercase();

    if up.contains("SLOT 1") || up.contains("TS1") || up.contains("SLOT1") {
        f.slot = Some(1);
    } else if up.contains("SLOT 2") || up.contains("TS2") || up.contains("SLOT2") {
        f.slot = Some(2);
    }

    f.cc = field_after(&up, &["COLOR CODE", "CC"]).map(String::from);
    f.tg = field_after(&up, &["TALKGROUP", "TGT", "TG"]).map(String::from);
    // source: RID (DMR), SRC (generic), MY (D-STAR)
    f.src = field_after(line, &["RID", "SRC", "Src", "MY"]).map(String::from);
    f.dst = field_after(line, &["DST", "UR", "Tgt"]).map(String::from);

    for kind in [
        "Group Call",
        "Private Call",
        "All Call",
        "Data Call",
        "Voice Call",
    ] {
        if line.contains(kind) || up.contains(&kind.to_ascii_uppercase()) {
            f.kind = Some(kind.to_string());
            break;
        }
    }

    if let Some(v) = field_after(&up, &["NAC"]) {
        f.extra.push(("NAC".into(), v.into()));
    }
    if let Some(v) = field_after(&up, &["RAN"]) {
        f.extra.push(("RAN".into(), v.into()));
    }
    if let Some(v) = field_after(&up, &["CAN"]) {
        f.extra.push(("CAN".into(), v.into()));
    }
    // dsd-neo prints "RPT 1:"/"RPT 2:" (with a space); DSD-FME uses "RPT1".
    if let Some(v) = field_after(line, &["RPT1", "RPT 1"]) {
        f.extra.push(("RPT1".into(), v.into()));
    }
    if let Some(v) = field_after(line, &["RPT2", "RPT 2"]) {
        f.extra.push(("RPT2".into(), v.into()));
    }

    // CC false-positives: "CC" inside words is guarded by token boundary,
    // but a bare "CC" hit that equals the TG value from "Color Code" parsing
    // ambiguity is acceptable noise — decoder formats vary wildly anyway.
    f
}

/// Is this line interesting enough to show in the event history?
#[allow(dead_code)] // used by tests; future event filtering
pub fn is_event_line(line: &str) -> bool {
    let f = parse_line(line);
    f.tg.is_some() || f.src.is_some() || f.kind.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmr_full_line() {
        let f =
            parse_line("Slot 2  TLC  DMR  | Color Code=01 | Group Call | TG: 91 | RID: 2621234");
        assert_eq!(f.slot, Some(2));
        assert_eq!(f.cc.as_deref(), Some("01"));
        assert_eq!(f.tg.as_deref(), Some("91"));
        assert_eq!(f.src.as_deref(), Some("2621234"));
        assert_eq!(f.kind.as_deref(), Some("Group Call"));
    }

    #[test]
    fn dmr_ts_variant() {
        let f = parse_line("DMR TS1 CC 3 TGT=2311 Src=1234567 Private Call");
        assert_eq!(f.slot, Some(1));
        assert_eq!(f.cc.as_deref(), Some("3"));
        assert_eq!(f.tg.as_deref(), Some("2311"));
        assert_eq!(f.src.as_deref(), Some("1234567"));
        assert_eq!(f.kind.as_deref(), Some("Private Call"));
    }

    #[test]
    fn ysf_and_dstar() {
        let f = parse_line("YSF DN | DST: ALL        | SRC: YU1ABC");
        assert_eq!(f.src.as_deref(), Some("YU1ABC"));
        assert_eq!(f.dst.as_deref(), Some("ALL"));

        let f = parse_line("DSTAR HD | UR: CQCQCQ | MY: YU1ABC/ID51 | RPT1: YU0RPT");
        assert_eq!(f.dst.as_deref(), Some("CQCQCQ"));
        assert_eq!(f.src.as_deref(), Some("YU1ABC/ID51"));
        assert!(f.extra.iter().any(|(k, v)| k == "RPT1" && v == "YU0RPT"));
    }

    #[test]
    fn p25_nac() {
        let f = parse_line("P25 LDU1 NAC: 293 TG: 10101 SRC: 5551234");
        assert!(f.extra.iter().any(|(k, v)| k == "NAC" && v == "293"));
        assert_eq!(f.tg.as_deref(), Some("10101"));
    }

    #[test]
    fn noise_is_boring() {
        assert!(!is_event_line("Sync: no sync"));
        assert!(!is_event_line("DSD-FME digital speech decoder"));
        assert!(is_event_line("TG: 91 RID: 1234"));
    }

    #[test]
    fn merge_keeps_latest() {
        let mut a = parse_line("Slot 2 | Color Code=01 | TG: 91");
        a.merge(parse_line("RID: 2621234 | Group Call"));
        assert_eq!(a.slot, Some(2));
        assert_eq!(a.src.as_deref(), Some("2621234"));
        assert_eq!(a.tg.as_deref(), Some("91"));
    }
}
