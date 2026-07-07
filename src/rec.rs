//! Minimal WAV (RIFF) writer for RX recordings: s16le mono, plus a scan of
//! the recordings folder for the in-app explorer.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One file in the recordings folder (WAV audio or raw IQ capture).
pub struct RecEntry {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    /// duration in seconds for WAV files; 0 for raw IQ
    pub secs: f32,
    pub modified: SystemTime,
    pub is_wav: bool,
    /// embedded RIFF INFO comment (mode/freq/callsign/TG), if any
    pub comment: Option<String>,
}

/// Duration of a mono WAV in seconds from its header (0 on any problem).
fn wav_seconds(path: &Path) -> f32 {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return 0.0,
    };
    let mut h = [0u8; 44];
    if f.read_exact(&mut h).is_err() || &h[..4] != b"RIFF" {
        return 0.0;
    }
    let rate = u32::from_le_bytes([h[24], h[25], h[26], h[27]]);
    let data = u32::from_le_bytes([h[40], h[41], h[42], h[43]]);
    if rate == 0 {
        0.0
    } else {
        data as f32 / (rate as f32 * 2.0)
    }
}

/// Read the RIFF INFO/ICMT comment from a WAV (walks chunks past `data`).
fn wav_comment(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let mut i = 12;
    while i + 8 <= data.len() {
        let id = &data[i..i + 4];
        let sz = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let body = i + 8;
        if id == b"LIST" && body + 4 <= data.len() && &data[body..body + 4] == b"INFO" {
            // scan sub-chunks for ICMT
            let mut j = body + 4;
            let end = (body + sz).min(data.len());
            while j + 8 <= end {
                let sid = &data[j..j + 4];
                let ssz = u32::from_le_bytes([data[j + 4], data[j + 5], data[j + 6], data[j + 7]])
                    as usize;
                if sid == b"ICMT" {
                    let t = &data[j + 8..(j + 8 + ssz).min(data.len())];
                    let s = String::from_utf8_lossy(t)
                        .trim_end_matches('\0')
                        .trim()
                        .to_string();
                    return (!s.is_empty()).then_some(s);
                }
                j += 8 + ssz + (ssz & 1);
            }
        }
        i = body + sz + (sz & 1);
    }
    None
}

/// List recordings (newest first). Covers .wav and the raw IQ extensions.
pub fn list_recordings(dir: &Path) -> Vec<RecEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let path = e.path();
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_wav = ext == "wav";
        if !is_wav && !matches!(ext.as_str(), "cu8" | "cs8" | "cs16" | "cf32") {
            continue;
        }
        let md = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        out.push(RecEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            secs: if is_wav { wav_seconds(&path) } else { 0.0 },
            bytes: md.len(),
            modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            comment: if is_wav { wav_comment(&path) } else { None },
            is_wav,
            path,
        });
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.modified));
    out
}

/// Human-readable byte size.
pub fn fmt_size(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.0} kB", bytes as f64 / (1u64 << 10) as f64)
    } else {
        format!("{bytes} B")
    }
}

pub struct WavWriter {
    f: File,
    data_bytes: u32,
    pub path: PathBuf,
}

impl WavWriter {
    pub fn create(path: &Path, rate: u32) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = File::create(path)?;
        let byte_rate = rate * 2;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&0u32.to_le_bytes()); // patched on finalize
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // PCM
        h.extend_from_slice(&1u16.to_le_bytes()); // mono
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&byte_rate.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes()); // block align
        h.extend_from_slice(&16u16.to_le_bytes()); // bits
        h.extend_from_slice(b"data");
        h.extend_from_slice(&0u32.to_le_bytes()); // patched on finalize
        f.write_all(&h)?;
        Ok(Self {
            f,
            data_bytes: 0,
            path: path.to_path_buf(),
        })
    }

    pub fn write_s16(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.f.write_all(bytes)?;
        self.data_bytes = self.data_bytes.saturating_add(bytes.len() as u32);
        Ok(())
    }

    #[allow(dead_code)] // used by tests; the UI derives elapsed from rec_since
    pub fn seconds(&self, rate: u32) -> f32 {
        self.data_bytes as f32 / (rate as f32 * 2.0)
    }

    /// Finalize, optionally embedding a RIFF INFO comment (ICMT) — used to
    /// tag digital-voice recordings with callsign/TG/mode/freq. Readable by
    /// ffprobe, mediainfo, exiftool, etc.
    pub fn finalize(mut self, comment: Option<&str>) -> std::io::Result<PathBuf> {
        let mut extra: u32 = 0;
        if let Some(c) = comment.filter(|c| !c.is_empty()) {
            self.f.seek(SeekFrom::End(0))?;
            let mut text = c.as_bytes().to_vec();
            text.push(0); // NUL-terminate
            if text.len() % 2 == 1 {
                text.push(0); // pad to even
            }
            let icmt_len = text.len() as u32;
            let list_size = 4 + 8 + icmt_len; // "INFO" + ("ICMT"+len) + text
            let mut buf = Vec::with_capacity(8 + list_size as usize);
            buf.extend_from_slice(b"LIST");
            buf.extend_from_slice(&list_size.to_le_bytes());
            buf.extend_from_slice(b"INFO");
            buf.extend_from_slice(b"ICMT");
            buf.extend_from_slice(&icmt_len.to_le_bytes());
            buf.extend_from_slice(&text);
            self.f.write_all(&buf)?;
            extra = 8 + list_size;
        }
        self.f.seek(SeekFrom::Start(4))?;
        self.f
            .write_all(&(36 + self.data_bytes + extra).to_le_bytes())?;
        self.f.seek(SeekFrom::Start(40))?;
        self.f.write_all(&self.data_bytes.to_le_bytes())?;
        self.f.flush()?;
        Ok(self.path)
    }
}

// ------------------------------------------------------------- WAV player

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// In-app WAV player with pause/seek. A feeder thread streams the PCM to an
/// audio sink from a shared byte cursor; play/pause/seek bump a generation
/// counter so the old feeder tears down its sink and a fresh one starts at
/// the new position (clean, flush-free seeking).
pub struct WavPlayer {
    pub path: PathBuf,
    data: Arc<Vec<u8>>,
    data_off: usize,
    rate: u32,
    pos: Arc<AtomicU64>, // byte offset into the data chunk
    playing: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    sink_cmd: String,
}

fn player_sink_cmd(rate: u32) -> Option<String> {
    if crate::pipeline::which("paplay").is_some() {
        Some(format!(
            "paplay --raw --rate={rate} --format=s16le --channels=1"
        ))
    } else if crate::pipeline::which("aplay").is_some() {
        Some(format!("aplay -q -t raw -f S16_LE -r {rate} -c 1 -"))
    } else {
        None
    }
}

/// Find the `data` chunk (offset, len) and sample rate in a WAV byte buffer.
fn wav_data_chunk(b: &[u8]) -> Option<(usize, usize, u32)> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut rate = 0u32;
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let sz = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = i + 8;
        if id == b"fmt " && body + 16 <= b.len() {
            rate = u32::from_le_bytes([b[body + 4], b[body + 5], b[body + 6], b[body + 7]]);
        } else if id == b"data" {
            return Some((body, sz.min(b.len().saturating_sub(body)), rate));
        }
        i = body + sz + (sz & 1);
    }
    None
}

impl WavPlayer {
    pub fn open(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let (off, len, rate) = wav_data_chunk(&bytes)?;
        let rate = if rate == 0 { 48_000 } else { rate };
        let sink_cmd = player_sink_cmd(rate)?;
        let mut data = bytes;
        data.truncate(off + len);
        let p = Self {
            path: path.to_path_buf(),
            data: Arc::new(data),
            data_off: off,
            rate,
            pos: Arc::new(AtomicU64::new(0)),
            playing: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            sink_cmd,
        };
        p.play();
        Some(p)
    }

    pub fn duration(&self) -> f32 {
        (self.data.len() - self.data_off) as f32 / (self.rate as f32 * 2.0)
    }

    pub fn position(&self) -> f32 {
        self.pos.load(Ordering::Relaxed) as f32 / (self.rate as f32 * 2.0)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed); // old feeder tears down
    }

    /// Seek to `secs` (clamped); resumes playing from there if it was playing.
    pub fn seek(&self, secs: f32) {
        let total = (self.data.len() - self.data_off) as f32;
        let mut b = (secs * self.rate as f32 * 2.0).clamp(0.0, total) as u64;
        b &= !1; // 2-byte align
        let was = self.is_playing();
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.pos.store(b, Ordering::Relaxed);
        if was {
            self.play();
        }
    }

    pub fn seek_by(&self, secs: f32) {
        self.seek((self.position() + secs).max(0.0));
    }

    fn play(&self) {
        let gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.playing.store(true, Ordering::Relaxed);
        let data = self.data.clone();
        let data_off = self.data_off;
        let pos = self.pos.clone();
        let playing = self.playing.clone();
        let generation = self.generation.clone();
        let cmd = self.sink_cmd.clone();
        // ~20 ms chunks, paced just under real time so the sink buffer stays
        // shallow and pause/seek respond promptly.
        let chunk = (self.rate as usize / 50 * 2).max(1920);
        std::thread::spawn(move || {
            let Ok(mut sink) = crate::pipeline::spawn_shell(&cmd, false, true) else {
                playing.store(false, Ordering::Relaxed);
                return;
            };
            let mut stdin = sink.child.stdin.take();
            let total = data.len();
            loop {
                if generation.load(Ordering::Relaxed) != gen {
                    break; // superseded by pause/seek/new play
                }
                let p = pos.load(Ordering::Relaxed) as usize + data_off;
                if p >= total {
                    playing.store(false, Ordering::Relaxed);
                    break;
                }
                let end = (p + chunk).min(total);
                let ok = stdin
                    .as_mut()
                    .map(|w| w.write_all(&data[p..end]).is_ok())
                    .unwrap_or(false);
                if !ok {
                    break;
                }
                pos.fetch_add((end - p) as u64, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(18));
            }
            drop(stdin);
            crate::pipeline::kill_group(sink.pgid);
        });
    }
}

impl Drop for WavPlayer {
    fn drop(&mut self) {
        self.pause();
    }
}

/// Where recordings land: config override → XDG music dir → ~/deck-recordings.
pub fn recordings_dir(cfg_dir: &str) -> PathBuf {
    if !cfg_dir.is_empty() {
        return PathBuf::from(shellexpand_home(cfg_dir));
    }
    dirs::audio_dir()
        .map(|d| d.join("deck"))
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("deck-recordings")
        })
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

pub fn recording_filename(mode_key: &str, freq_hz: u64) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("deck_{mode_key}_{}_{ts}.wav", crate::freq::fmt_mhz(freq_hz))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_lists_and_reads_duration() {
        let tmp = std::env::temp_dir().join(format!("deck-rectest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mut w = WavWriter::create(&tmp.join("a.wav"), 48_000).unwrap();
        w.write_s16(&vec![0u8; 48_000 * 2]).unwrap(); // 1.0 s
        w.finalize(None).unwrap();
        std::fs::write(tmp.join("b.cu8"), vec![0u8; 4096]).unwrap();
        std::fs::write(tmp.join("skip.txt"), b"x").unwrap();
        let rows = list_recordings(&tmp);
        assert_eq!(rows.len(), 2, "wav + cu8, txt skipped");
        let wav = rows.iter().find(|r| r.name == "a.wav").unwrap();
        assert!((wav.secs - 1.0).abs() < 0.01);
        assert!(wav.is_wav);
        let iq = rows.iter().find(|r| r.name == "b.cu8").unwrap();
        assert!(!iq.is_wav);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn wav_header_is_valid() {
        let p = std::env::temp_dir().join(format!("deck-test-{}.wav", std::process::id()));
        let mut w = WavWriter::create(&p, 48_000).unwrap();
        let samples: Vec<u8> = (0..1000i16).flat_map(|v| v.to_le_bytes()).collect();
        w.write_s16(&samples).unwrap();
        assert!((w.seconds(48_000) - 1000.0 / 48_000.0).abs() < 1e-6);
        let path = w.finalize(Some("deck test src=YT7OP tg=91")).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(riff as usize, bytes.len() - 8);
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert_eq!(data, 2000);
        let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        assert_eq!(rate, 48_000);
        // embedded RIFF INFO comment reads back
        assert_eq!(
            wav_comment(&path).as_deref(),
            Some("deck test src=YT7OP tg=91")
        );
        let _ = std::fs::remove_file(path);
    }
}
