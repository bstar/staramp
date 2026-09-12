//! Minimal WAV writer.
//!
//! Exists so `staramp decode` can dump a decode to disk and be byte-compared
//! against ffmpeg's output. Proving sample accuracy before any audio hardware is
//! involved is much cheaper than debugging it through a sound server.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result};

/// 32-bit float PCM writer. Float because that is what the decoders produce, and
/// converting to 16-bit here would make the comparison test prove the wrong thing.
pub struct WavWriter {
    out: BufWriter<File>,
    channels: u16,
    data_bytes: u32,
}

impl WavWriter {
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
        let mut w = Self {
            out: BufWriter::new(file),
            channels,
            data_bytes: 0,
        };
        w.write_header(sample_rate)?;
        Ok(w)
    }

    fn write_header(&mut self, sample_rate: u32) -> Result<()> {
        let ch = self.channels;
        let bits = 32u16;
        let block_align = ch * bits / 8;
        let byte_rate = sample_rate * block_align as u32;

        let o = &mut self.out;
        o.write_all(b"RIFF")?;
        o.write_all(&0u32.to_le_bytes())?; // patched on finish
        o.write_all(b"WAVE")?;

        o.write_all(b"fmt ")?;
        o.write_all(&16u32.to_le_bytes())?;
        o.write_all(&3u16.to_le_bytes())?; // WAVE_FORMAT_IEEE_FLOAT
        o.write_all(&ch.to_le_bytes())?;
        o.write_all(&sample_rate.to_le_bytes())?;
        o.write_all(&byte_rate.to_le_bytes())?;
        o.write_all(&block_align.to_le_bytes())?;
        o.write_all(&bits.to_le_bytes())?;

        o.write_all(b"data")?;
        o.write_all(&0u32.to_le_bytes())?; // patched on finish
        Ok(())
    }

    /// Append interleaved samples.
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        // A stack buffer keeps this off the allocator on the hot path.
        let mut chunk = [0u8; 4096];
        for block in samples.chunks(chunk.len() / 4) {
            let n = block.len() * 4;
            for (i, s) in block.iter().enumerate() {
                chunk[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
            }
            self.out.write_all(&chunk[..n])?;
        }
        self.data_bytes += (samples.len() * 4) as u32;
        Ok(())
    }

    /// Patch the two length fields. Not calling this leaves an unreadable file,
    /// so it consumes `self`.
    pub fn finish(mut self) -> Result<()> {
        self.out.flush()?;
        let f = self.out.get_mut();
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&(36 + self.data_bytes).to_le_bytes())?;
        f.seek(SeekFrom::Start(40))?;
        f.write_all(&self.data_bytes.to_le_bytes())?;
        f.flush()?;
        Ok(())
    }
}
