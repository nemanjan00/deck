//! Minimal WAV (RIFF) writer for RX recordings: s16le mono.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

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

    pub fn finalize(mut self) -> std::io::Result<PathBuf> {
        self.f.seek(SeekFrom::Start(4))?;
        self.f.write_all(&(36 + self.data_bytes).to_le_bytes())?;
        self.f.seek(SeekFrom::Start(40))?;
        self.f.write_all(&self.data_bytes.to_le_bytes())?;
        self.f.flush()?;
        Ok(self.path)
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
    fn wav_header_is_valid() {
        let p = std::env::temp_dir().join(format!("deck-test-{}.wav", std::process::id()));
        let mut w = WavWriter::create(&p, 48_000).unwrap();
        let samples: Vec<u8> = (0..1000i16).flat_map(|v| v.to_le_bytes()).collect();
        w.write_s16(&samples).unwrap();
        assert!((w.seconds(48_000) - 1000.0 / 48_000.0).abs() < 1e-6);
        let path = w.finalize().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(riff as usize, bytes.len() - 8);
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert_eq!(data, 2000);
        let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        assert_eq!(rate, 48_000);
        let _ = std::fs::remove_file(path);
    }
}
