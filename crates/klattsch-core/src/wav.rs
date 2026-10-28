// Minimal RIFF/WAVE encoder w/ normalization. Optionally embeds a LIST INFO
// chunk after the data chunk with ISFT (software identifier) and ICMT (free-form
// comment, used here to round-trip the source utterance string).

use crate::js;

// The ISFT (software) value klattsch's command-line tools write.
pub const SOFTWARE: &str = "klattsch · https://tgies.github.io/klattsch";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WavMetadata {
    pub software: Option<String>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WavOptions {
    // Peak level to normalize to; `None`, 0 or NaN disables normalization.
    pub peak_normalize: Option<f64>,
    pub metadata: Option<WavMetadata>,
}

impl Default for WavOptions {
    fn default() -> Self {
        Self { peak_normalize: Some(0.95), metadata: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodedWav {
    pub bytes: Vec<u8>,
    pub gain: f64,
}

fn build_info_chunk(metadata: &WavMetadata) -> Option<Vec<u8>> {
    let mut subs: Vec<(&[u8; 4], &[u8])> = Vec::new();
    if let Some(s) = metadata.software.as_deref().filter(|s| !s.is_empty()) {
        subs.push((b"ISFT", s.as_bytes()));
    }
    if let Some(s) = metadata.comment.as_deref().filter(|s| !s.is_empty()) {
        subs.push((b"ICMT", s.as_bytes()));
    }
    if subs.is_empty() {
        return None;
    }
    // LIST payload = 'INFO' fourcc (4 bytes) + each sub-chunk (8-byte header
    // + data + 1 byte of padding to keep chunks word-aligned when odd-sized).
    let mut payload_size = 4usize;
    for (_, data) in &subs {
        payload_size += 8 + data.len() + data.len() % 2;
    }
    let mut out = Vec::with_capacity(8 + payload_size);
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&(payload_size as u32).to_le_bytes());
    out.extend_from_slice(b"INFO");
    for (id, data) in &subs {
        out.extend_from_slice(*id);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
    }
    Some(out)
}

pub fn encode_wav(samples: &[f32], sample_rate: u32, opts: &WavOptions) -> EncodedWav {
    let mut gain = 1.0;
    if let Some(peak_normalize) = opts.peak_normalize.filter(|p| *p != 0.0 && !p.is_nan()) {
        let mut peak = 0.0f64;
        for &s in samples {
            let s = f64::from(s);
            let a = if s < 0.0 { -s } else { s };
            if a > peak {
                peak = a;
            }
        }
        if peak > 0.0 {
            gain = peak_normalize / peak;
        }
    }

    let data_bytes = samples.len() * 2;
    let info = opts.metadata.as_ref().and_then(build_info_chunk);
    let total_size = 44 + data_bytes + info.as_ref().map_or(0, Vec::len);
    let mut out = Vec::with_capacity(total_size);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((total_size - 8) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format = PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&sample_rate.wrapping_mul(2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());

    for &s in samples {
        let mut s = f64::from(s) * gain;
        if s > 1.0 {
            s = 1.0;
        } else if s < -1.0 {
            s = -1.0;
        }
        let r = js::round(s * 32767.0);
        let v = if r.is_nan() { 0 } else { r as i16 };
        out.extend_from_slice(&v.to_le_bytes());
    }

    if let Some(info) = info {
        out.extend_from_slice(&info);
    }

    EncodedWav { bytes: out, gain }
}
