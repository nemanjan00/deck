//! multimon-ng output: POCSAG pages and AFSK1200 (APRS) packets.

#[derive(Clone, Debug, PartialEq)]
pub enum PagerContent {
    Alpha(String),
    Numeric(String),
    ToneOnly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PagerMsg {
    pub baud: u32,
    pub address: String,
    pub function: String,
    pub content: PagerContent,
}

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// `POCSAG1200: Address: 1234567  Function: 0  Alpha:   hello world`
pub fn parse_pocsag(line: &str) -> Option<PagerMsg> {
    let rest = line.strip_prefix("POCSAG")?;
    let (baud, rest) = rest.split_once(':')?;
    let baud: u32 = baud.trim().parse().ok()?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("Address:")?.trim_start();
    let (address, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let rest = rest.trim_start();
    let (function, rest) = match rest.strip_prefix("Function:") {
        Some(r) => {
            let r = r.trim_start();
            r.split_once(char::is_whitespace).unwrap_or((r, ""))
        }
        None => ("", rest),
    };
    let rest = rest.trim_start();
    let content = if let Some(i) = rest.find("Alpha:") {
        PagerContent::Alpha(clean(&rest[i + 6..]))
    } else if let Some(i) = rest.find("Numeric:") {
        PagerContent::Numeric(clean(&rest[i + 8..]))
    } else {
        PagerContent::ToneOnly
    };
    Some(PagerMsg {
        baud,
        address: address.to_string(),
        function: function.to_string(),
        content,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct AprsMsg {
    pub from: String,
    pub to: String,
    pub path: String,
    pub info: String,
}

/// AFSK1200 prints a header line, then the payload on the next line:
/// `AFSK1200: fm N0CALL-9 to APDECK-0 via WIDE1-1,WIDE2-2 UI pid=F0`
/// `!4447.40N/02027.60E>deck sim`
#[derive(Default)]
pub struct AprsParser {
    pending: Option<AprsMsg>,
}

impl AprsParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, line: &str) -> Option<AprsMsg> {
        if let Some(rest) = line.strip_prefix("AFSK1200: fm ") {
            // starting a new header flushes any headerless previous one
            let mut toks = rest.split_whitespace();
            let from = toks.next().unwrap_or("").to_string();
            let mut to = String::new();
            let mut path = String::new();
            let mut prev = "";
            for t in rest.split_whitespace() {
                match prev {
                    "to" => to = t.to_string(),
                    "via" => path = t.to_string(),
                    _ => {}
                }
                prev = t;
            }
            self.pending = Some(AprsMsg {
                from,
                to,
                path,
                info: String::new(),
            });
            return None;
        }
        if line.starts_with("AFSK1200:") {
            return None;
        }
        if let Some(mut msg) = self.pending.take() {
            let payload = clean(line);
            if payload.is_empty() {
                self.pending = Some(msg);
                return None;
            }
            msg.info = payload;
            return Some(msg);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pocsag_alpha() {
        let m = parse_pocsag(
            "POCSAG1200: Address: 1234567  Function: 0  Alpha:   A2 Ambulancepost Aalsmeer",
        )
        .unwrap();
        assert_eq!(m.baud, 1200);
        assert_eq!(m.address, "1234567");
        assert_eq!(m.function, "0");
        assert_eq!(
            m.content,
            PagerContent::Alpha("A2 Ambulancepost Aalsmeer".into())
        );
    }

    #[test]
    fn pocsag_numeric_and_tone() {
        let m =
            parse_pocsag("POCSAG512: Address: 200042  Function: 3  Numeric: 0612345678").unwrap();
        assert_eq!(m.baud, 512);
        assert_eq!(m.content, PagerContent::Numeric("0612345678".into()));

        let m = parse_pocsag("POCSAG2400: Address: 88  Function: 1").unwrap();
        assert_eq!(m.content, PagerContent::ToneOnly);
    }

    #[test]
    fn pocsag_rejects_other_lines() {
        assert!(parse_pocsag("AFSK1200: fm X to Y UI").is_none());
        assert!(parse_pocsag("random noise").is_none());
    }

    #[test]
    fn aprs_two_line() {
        let mut p = AprsParser::new();
        assert!(p
            .push("AFSK1200: fm YU1ABC-9 to APDECK-0 via WIDE1-1,WIDE2-2 UI pid=F0")
            .is_none());
        let m = p.push("!4447.40N/02027.60E>deck rocks").unwrap();
        assert_eq!(m.from, "YU1ABC-9");
        assert_eq!(m.to, "APDECK-0");
        assert_eq!(m.path, "WIDE1-1,WIDE2-2");
        assert_eq!(m.info, "!4447.40N/02027.60E>deck rocks");
        // stray payload without header is ignored
        assert!(p.push("orphan line").is_none());
    }
}
