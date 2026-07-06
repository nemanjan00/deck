//! Decoded-line simulation for digital voice modes: emits dsd-neo-flavoured
//! text so the call card, history and parsers get exercised without RF.

use crate::dsp::Rng;

pub struct VoiceSim {
    rng: Rng,
    proto: String,
    callsigns: Vec<String>,
}

pub struct SimLine {
    pub delay_ms: u64,
    pub text: String,
}

impl VoiceSim {
    pub fn new(proto: &str, seed: u64) -> Self {
        Self {
            rng: Rng::new(seed ^ 0x701CE),
            proto: proto.to_string(),
            callsigns: ["YU1ABC", "YT2XYZ", "S52DK", "9A3W", "OE7DK", "HA5K"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    /// One complete call: setup, voice frames, teardown.
    pub fn call(&mut self) -> Vec<SimLine> {
        let mut out = Vec::new();
        let frames = self.rng.range_u32(6, 22);
        match self.proto.as_str() {
            "dmr" => {
                let slot = 1 + (self.rng.next_u64() & 1);
                let cc = self.rng.range_u32(1, 15);
                let tg = *self.rng.pick(&[9u32, 91, 214, 2311, 26212, 260210]);
                let rid = 2_620_000 + self.rng.range_u32(0, 9999);
                let kind = if self.rng.f64() < 0.85 {
                    "Group Call"
                } else {
                    "Private Call"
                };
                out.push(SimLine {
                    delay_ms: 0,
                    text: format!("Sync: +DMR  [slot{slot}]  | Color Code={cc:02} | VLC"),
                });
                out.push(SimLine {
                    delay_ms: 60,
                    text: format!(
                        "Slot {slot} TLC DMR | Color Code={cc:02} | {kind} | TG: {tg} | RID: {rid}"
                    ),
                });
                for i in 0..frames {
                    out.push(SimLine {
                        delay_ms: 360,
                        text: format!(
                            "Slot {slot} VC{} DMR | Color Code={cc:02} | {kind} | TG: {tg} | RID: {rid}",
                            1 + i % 6
                        ),
                    });
                }
                out.push(SimLine {
                    delay_ms: 200,
                    text: format!("Slot {slot} TLC DMR | Color Code={cc:02} | Call End | TG: {tg}"),
                });
            }
            "ysf" => {
                let src = self.rng.pick(&self.callsigns).clone();
                let dst = if self.rng.f64() < 0.8 { "ALL" } else { "CQCQCQ" };
                out.push(SimLine {
                    delay_ms: 0,
                    text: "Sync: +YSF VD2".into(),
                });
                for _ in 0..frames {
                    out.push(SimLine {
                        delay_ms: 350,
                        text: format!("YSF VD2 | DN | DST: {dst:<8} | SRC: {src:<8} | UL: DIRECT"),
                    });
                }
                out.push(SimLine {
                    delay_ms: 150,
                    text: "YSF EOT".into(),
                });
            }
            "dstar" => {
                let my = self.rng.pick(&self.callsigns).clone();
                out.push(SimLine {
                    delay_ms: 0,
                    text: "Sync: +DSTAR HD".into(),
                });
                for _ in 0..frames {
                    out.push(SimLine {
                        delay_ms: 350,
                        text: format!(
                            "DSTAR | UR: CQCQCQ   | MY: {my}/ID51 | RPT1: YU0RPT B | RPT2: YU0RPT G"
                        ),
                    });
                }
                out.push(SimLine {
                    delay_ms: 150,
                    text: "DSTAR EOT".into(),
                });
            }
            "nxdn" => {
                let ran = self.rng.range_u32(1, 63);
                let tg = self.rng.range_u32(100, 999);
                let src = self.rng.range_u32(1000, 9999);
                for _ in 0..frames {
                    out.push(SimLine {
                        delay_ms: 350,
                        text: format!(
                            "NXDN48 VCH | RAN: {ran} | Group Call | TG: {tg} | SRC: {src}"
                        ),
                    });
                }
            }
            "p25" => {
                let nac = format!("{:03X}", self.rng.range_u32(0x100, 0xFFF));
                let tg = self.rng.range_u32(10000, 65000);
                let src = self.rng.range_u32(1_000_000, 9_999_999);
                out.push(SimLine {
                    delay_ms: 0,
                    text: format!("P25 HDU | NAC: {nac}"),
                });
                for i in 0..frames {
                    out.push(SimLine {
                        delay_ms: 360,
                        text: format!(
                            "P25 LDU{} | NAC: {nac} | Group Call | TG: {tg} | SRC: {src}",
                            1 + i % 2
                        ),
                    });
                }
                out.push(SimLine {
                    delay_ms: 150,
                    text: format!("P25 TDU | NAC: {nac}"),
                });
            }
            _ => {
                // m17 and anything new
                let src = self.rng.pick(&self.callsigns).clone();
                for _ in 0..frames {
                    out.push(SimLine {
                        delay_ms: 350,
                        text: format!("M17 STR | DST: ALL       | SRC: {src:<9} | CAN: 0"),
                    });
                }
            }
        }
        out
    }

    pub fn idle_gap_ms(&mut self) -> u64 {
        u64::from(self.rng.range_u32(1200, 5000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::dsd;

    #[test]
    fn sim_lines_satisfy_the_parser() {
        for proto in ["dmr", "ysf", "dstar", "nxdn", "p25", "m17"] {
            let mut sim = VoiceSim::new(proto, 3);
            let lines = sim.call();
            assert!(!lines.is_empty());
            let mut merged = dsd::CallFields::default();
            for l in &lines {
                merged.merge(dsd::parse_line(&l.text));
            }
            assert!(
                merged.tg.is_some() || merged.src.is_some(),
                "{proto}: no TG/SRC extracted"
            );
            if proto == "dmr" {
                assert!(merged.slot.is_some(), "dmr slot");
                assert!(merged.cc.is_some(), "dmr color code");
                assert!(merged.kind.is_some(), "dmr call kind");
            }
        }
    }
}
