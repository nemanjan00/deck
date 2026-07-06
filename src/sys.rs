//! Handheld/system integration: battery, master volume, power actions.
//! deck is the fullscreen UI of the device, so it surfaces these itself.
//! Everything degrades to "hidden" on machines without the corresponding
//! facility (no battery, no mixer, no systemd).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatState {
    Discharging,
    Charging,
    Full,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct Battery {
    pub percent: u8,
    pub state: BatState,
}

fn read_battery_in(root: &Path) -> Option<Battery> {
    let rd = std::fs::read_dir(root).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        let type_ok = std::fs::read_to_string(p.join("type"))
            .map(|t| t.trim() == "Battery")
            .unwrap_or(false);
        if !type_ok {
            continue;
        }
        let cap = std::fs::read_to_string(p.join("capacity")).ok()?;
        let percent: u8 = cap.trim().parse().ok()?;
        let state = match std::fs::read_to_string(p.join("status"))
            .unwrap_or_default()
            .trim()
        {
            "Charging" => BatState::Charging,
            "Discharging" => BatState::Discharging,
            "Full" => BatState::Full,
            _ => BatState::Unknown,
        };
        return Some(Battery { percent, state });
    }
    None
}

pub fn read_battery() -> Option<Battery> {
    read_battery_in(Path::new("/sys/class/power_supply"))
}

/// Which mixer CLI we talk to for the master volume.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MixerKind {
    Wpctl,
    Pactl,
    Amixer,
}

pub struct SysMon {
    pub battery: Option<Battery>,
    pub volume: Option<u8>,
    pub muted: bool,
    mixer: Option<(MixerKind, PathBuf)>,
    last_bat: Instant,
    last_vol: Instant,
}

fn run_out(bin: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

impl SysMon {
    pub fn new() -> Self {
        let mixer = [
            ("wpctl", MixerKind::Wpctl),
            ("pactl", MixerKind::Pactl),
            ("amixer", MixerKind::Amixer),
        ]
        .iter()
        .find_map(|(bin, kind)| crate::pipeline::which(bin).map(|p| (*kind, p)));
        let mut s = Self {
            battery: None,
            volume: None,
            muted: false,
            mixer,
            last_bat: Instant::now() - Duration::from_secs(3600),
            last_vol: Instant::now() - Duration::from_secs(3600),
        };
        s.refresh(true);
        s
    }

    /// Cheap periodic refresh; call from the UI tick.
    pub fn refresh(&mut self, force: bool) {
        if force || self.last_bat.elapsed() > Duration::from_secs(10) {
            self.battery = read_battery();
            self.last_bat = Instant::now();
        }
        if force || self.last_vol.elapsed() > Duration::from_secs(20) {
            self.read_volume();
            self.last_vol = Instant::now();
        }
    }

    fn read_volume(&mut self) {
        let Some((kind, bin)) = self.mixer.clone() else {
            return;
        };
        match kind {
            MixerKind::Wpctl => {
                // "Volume: 0.65 [MUTED]"
                if let Some(out) = run_out(&bin, &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
                    self.muted = out.contains("MUTED");
                    if let Some(v) = out
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<f32>().ok())
                    {
                        self.volume = Some((v * 100.0).round().clamp(0.0, 150.0) as u8);
                    }
                }
            }
            MixerKind::Pactl => {
                if let Some(out) = run_out(&bin, &["get-sink-volume", "@DEFAULT_SINK@"]) {
                    // "... 65% ..."
                    if let Some(pct) = out.split('%').next().and_then(|s| {
                        s.rsplit(|c: char| !c.is_ascii_digit())
                            .next()
                            .and_then(|d| d.parse::<u8>().ok())
                    }) {
                        self.volume = Some(pct);
                    }
                }
                if let Some(out) = run_out(&bin, &["get-sink-mute", "@DEFAULT_SINK@"]) {
                    self.muted = out.contains("yes");
                }
            }
            MixerKind::Amixer => {
                if let Some(out) = run_out(&bin, &["get", "Master"]) {
                    // "[65%] [on]"
                    if let Some(i) = out.find('[') {
                        let rest = &out[i + 1..];
                        if let Some(j) = rest.find('%') {
                            self.volume = rest[..j].parse::<u8>().ok();
                        }
                    }
                    self.muted = out.contains("[off]");
                }
            }
        }
    }

    pub fn volume_step(&mut self, delta: i8) {
        let Some((kind, bin)) = self.mixer.clone() else {
            return;
        };
        let arg = if delta > 0 { "5%+" } else { "5%-" };
        match kind {
            MixerKind::Wpctl => {
                let _ = run_out(
                    &bin,
                    &["set-volume", "-l", "1.2", "@DEFAULT_AUDIO_SINK@", arg],
                );
            }
            MixerKind::Pactl => {
                let sign = if delta > 0 { "+5%" } else { "-5%" };
                let _ = run_out(&bin, &["set-sink-volume", "@DEFAULT_SINK@", sign]);
            }
            MixerKind::Amixer => {
                let a = if delta > 0 { "5%+" } else { "5%-" };
                let _ = run_out(&bin, &["-q", "set", "Master", a]);
            }
        }
        self.read_volume();
        self.last_vol = Instant::now();
    }

    pub fn toggle_mute(&mut self) {
        let Some((kind, bin)) = self.mixer.clone() else {
            return;
        };
        match kind {
            MixerKind::Wpctl => {
                let _ = run_out(&bin, &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
            }
            MixerKind::Pactl => {
                let _ = run_out(&bin, &["set-sink-mute", "@DEFAULT_SINK@", "toggle"]);
            }
            MixerKind::Amixer => {
                let _ = run_out(&bin, &["-q", "set", "Master", "toggle"]);
            }
        }
        self.read_volume();
    }
}

impl Default for SysMon {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerAction {
    PowerOff,
    Reboot,
    Suspend,
}

impl PowerAction {
    /// Executed via systemd-logind (works unprivileged on handheld images).
    pub fn execute(self) -> Result<(), String> {
        let verb = match self {
            PowerAction::PowerOff => "poweroff",
            PowerAction::Reboot => "reboot",
            PowerAction::Suspend => "suspend",
        };
        match Command::new("systemctl").arg(verb).status() {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("systemctl {verb} exited with {s}")),
            Err(e) => Err(format!("systemctl {verb}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_parse() {
        let tmp = std::env::temp_dir().join(format!("deck-test-bat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let bat = tmp.join("BAT0");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::write(bat.join("type"), "Battery\n").unwrap();
        std::fs::write(bat.join("capacity"), "87\n").unwrap();
        std::fs::write(bat.join("status"), "Charging\n").unwrap();
        let ac = tmp.join("AC");
        std::fs::create_dir_all(&ac).unwrap();
        std::fs::write(ac.join("type"), "Mains\n").unwrap();

        let b = read_battery_in(&tmp).unwrap();
        assert_eq!(b.percent, 87);
        assert_eq!(b.state, BatState::Charging);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
