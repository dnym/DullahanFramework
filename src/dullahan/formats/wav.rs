//! Zero-dependency RIFF/WAVE loader (PCM 16-bit & IEEE float32, mono & stereo)
//! with linear resampling to the engine mixer's native 48 kHz stereo `f32` format.

use std::fs;
use std::path::Path;

pub const TARGET_SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Debug)]
pub struct AudioClip {
    pub sample_rate: u32,
    /// Interleaved stereo frames `[left, right]` in `[-1.0, 1.0]`.
    pub frames: Vec<[f32; 2]>,
    pub duration: f32,
}

pub fn load_wav_file<P: AsRef<Path>>(path: P) -> Result<AudioClip, String> {
    let bytes = fs::read(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    decode_wav(&bytes)
}

pub fn decode_wav(bytes: &[u8]) -> Result<AudioClip, String> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }

    let mut pos = 12usize;
    let mut audio_fmt: u16 = 0;
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut data_slice: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        if pos + size > bytes.len() {
            return Err("truncated WAV chunk".into());
        }
        let chunk = &bytes[pos..pos + size];
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err("WAV fmt chunk too small".into());
                }
                audio_fmt = u16::from_le_bytes([chunk[0], chunk[1]]);
                channels = u16::from_le_bytes([chunk[2], chunk[3]]);
                sample_rate = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                bits_per_sample = u16::from_le_bytes([chunk[14], chunk[15]]);
            }
            b"data" => {
                data_slice = Some(chunk);
            }
            _ => {}
        }
        pos += size + (size & 1); // word-aligned chunks
    }

    let data = data_slice.ok_or("WAV missing data chunk")?;
    if channels == 0 || channels > 2 || sample_rate == 0 {
        return Err(format!("unsupported WAV channels={channels} sr={sample_rate}"));
    }

    let src_frames: Vec<[f32; 2]> = match (audio_fmt, bits_per_sample) {
        (1, 16) => {
            let step = (channels as usize) * 2;
            let count = data.len() / step;
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let off = i * step;
                let l = i16::from_le_bytes([data[off], data[off + 1]]) as f32 / 32768.0;
                let r = if channels == 2 {
                    i16::from_le_bytes([data[off + 2], data[off + 3]]) as f32 / 32768.0
                } else {
                    l
                };
                out.push([l, r]);
            }
            out
        }
        (3, 32) => {
            let step = (channels as usize) * 4;
            let count = data.len() / step;
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let off = i * step;
                let l = f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let r = if channels == 2 {
                    f32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap())
                } else {
                    l
                };
                out.push([l, r]);
            }
            out
        }
        _ => {
            return Err(format!(
                "unsupported WAV format={audio_fmt} bits={bits_per_sample} (supported: PCM16, Float32)"
            ));
        }
    };

    let frames = if sample_rate == TARGET_SAMPLE_RATE || src_frames.is_empty() {
        src_frames
    } else {
        resample_linear(&src_frames, sample_rate, TARGET_SAMPLE_RATE)
    };
    let duration = frames.len() as f32 / TARGET_SAMPLE_RATE as f32;

    Ok(AudioClip {
        sample_rate: TARGET_SAMPLE_RATE,
        frames,
        duration,
    })
}

fn resample_linear(src: &[[f32; 2]], src_sr: u32, dst_sr: u32) -> Vec<[f32; 2]> {
    let ratio = src_sr as f64 / dst_sr as f64;
    let dst_len = ((src.len() as f64) / ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(dst_len);
    let last = src.len() - 1;
    for i in 0..dst_len {
        let pos = i as f64 * ratio;
        let idx = (pos as usize).min(last);
        let next = (idx + 1).min(last);
        let frac = (pos - idx as f64) as f32;
        let a = src[idx];
        let b = src[next];
        out.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_minimal_pcm16_wav() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + 4).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&96_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&16384i16.to_le_bytes());
        wav.extend_from_slice(&(-16384i16).to_le_bytes());

        let clip = decode_wav(&wav).unwrap();
        assert_eq!(clip.frames.len(), 2);
        assert!((clip.frames[0][0] - 0.5).abs() < 1e-3);
        assert!((clip.frames[1][1] + 0.5).abs() < 1e-3);
    }
}
