use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Mutex;

use space_lt_common::debug;

/// Trait abstracting TTS synthesis. Returns 16kHz mono i16 samples.
pub trait TtsEngine: Send {
    fn synthesize(&self, text: &str) -> Result<Vec<i16>>;
}

/// Kokoro TTS engine via sherpa-rs (sherpa-onnx FFI).
/// Uses a Mutex because sherpa-rs KokoroTts::create() requires &mut self,
/// while our TtsEngine trait uses &self.
pub struct KokoroTts {
    tts: Mutex<sherpa_rs::tts::KokoroTts>,
    speaker_id: i32,
}

impl KokoroTts {
    /// Load Kokoro TTS from a model directory (e.g. kokoro-multi-lang-v1_0/).
    ///
    /// Expected directory contents:
    /// - model.onnx — ONNX neural network
    /// - voices.bin — speaker embeddings
    /// - tokens.txt — token vocabulary
    /// - espeak-ng-data/ — phoneme data
    /// - dict/ — dictionary data
    /// - lexicon-us-en.txt — English lexicon
    pub fn new(model_dir: &Path, lang: &str) -> Result<Self> {
        let model_dir_str = model_dir
            .to_str()
            .context("model directory path is not valid UTF-8")?;

        let model_path = model_dir.join("model.onnx");
        if !model_path.exists() {
            anyhow::bail!("model.onnx not found in {}", model_dir.display());
        }

        let voices_path = model_dir.join("voices.bin");
        if !voices_path.exists() {
            anyhow::bail!("voices.bin not found in {}", model_dir.display());
        }

        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            anyhow::bail!("tokens.txt not found in {}", model_dir.display());
        }

        debug!("[server] Loading TTS model from {}", model_dir.display());

        // Build lexicon: include all lexicon-*.txt files found in the directory
        let lexicon = build_lexicon_path(model_dir);

        let config = sherpa_rs::tts::KokoroTtsConfig {
            model: format!("{model_dir_str}/model.onnx"),
            voices: format!("{model_dir_str}/voices.bin"),
            tokens: format!("{model_dir_str}/tokens.txt"),
            data_dir: format!("{model_dir_str}/espeak-ng-data"),
            dict_dir: format!("{model_dir_str}/dict"),
            lexicon,
            length_scale: 1.0,
            lang: lang.to_string(),
            ..Default::default()
        };

        let tts = sherpa_rs::tts::KokoroTts::new(config);

        // Log model file size as proxy for memory usage (VRAM not directly queryable via sherpa-rs)
        let model_size_mb = std::fs::metadata(&model_path)
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);
        debug!(
            "[server] TTS model loaded ({:.0} MB on disk)",
            model_size_mb
        );

        Ok(Self {
            tts: Mutex::new(tts),
            speaker_id: 0, // default: first voice (af_alloy)
        })
    }
}

impl TtsEngine for KokoroTts {
    fn synthesize(&self, text: &str) -> Result<Vec<i16>> {
        let mut tts = self
            .tts
            .lock()
            .map_err(|e| anyhow::anyhow!("TTS mutex poisoned: {e}"))?;

        let audio = tts
            .create(text, self.speaker_id, 1.0)
            .map_err(|e| anyhow::anyhow!("TTS synthesis failed: {e}"))?;

        debug!(
            "[server] TTS synthesized {}ms audio at {}Hz",
            audio.duration, audio.sample_rate
        );

        // Resample 24kHz -> 16kHz
        let resampled = resample_24k_to_16k(&audio.samples)?;

        // Convert f32 -> i16 (clamp to [-1.0, 1.0], scale by i16::MAX)
        let samples: Vec<i16> = resampled
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();

        Ok(samples)
    }
}

/// Resample f32 audio from 24kHz to 16kHz mono.
fn resample_24k_to_16k(input: &[f32]) -> Result<Vec<f32>> {
    use audioadapter_buffers::direct::SequentialSliceOfVecs;
    use rubato::{
        Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
        WindowFunction,
    };

    let ratio = 16000.0 / 24000.0;
    let chunk_size = 1024;

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let mut resampler =
        Async::<f64>::new_sinc(ratio, 1.1, &params, chunk_size, 1, FixedAsync::Input)
            .context("creating 24kHz→16kHz resampler")?;

    let input_f64: Vec<f64> = input.iter().map(|&s| s as f64).collect();
    let mut output_all: Vec<f32> = Vec::new();
    let mut offset = 0;

    while offset < input_f64.len() {
        let end = (offset + chunk_size).min(input_f64.len());
        let chunk = &input_f64[offset..end];

        let padded: Vec<f64>;
        let input_slice: &[f64] = if chunk.len() < chunk_size {
            padded = {
                let mut v = chunk.to_vec();
                v.resize(chunk_size, 0.0);
                v
            };
            &padded
        } else {
            chunk
        };

        let input_data: Vec<Vec<f64>> = vec![input_slice.to_vec()];
        let adapter = SequentialSliceOfVecs::new(&input_data, 1, chunk_size)
            .context("creating resampler input adapter")?;

        match resampler.process(&adapter, 0, None) {
            Ok(output) => {
                let samples: Vec<f64> = output.take_data();
                let actual_out = if chunk.len() < chunk_size {
                    let expected = (chunk.len() as f64 * ratio).ceil() as usize;
                    &samples[..expected.min(samples.len())]
                } else {
                    &samples[..]
                };
                for &s in actual_out {
                    output_all.push(s as f32);
                }
            }
            Err(e) => {
                anyhow::bail!("Resample error: {e}");
            }
        }

        offset = end;
    }

    Ok(output_all)
}

/// Build comma-separated lexicon path from all lexicon-*.txt files in the directory.
fn build_lexicon_path(model_dir: &Path) -> String {
    let mut paths: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(model_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("lexicon-") && name_str.ends_with(".txt") {
                paths.push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    paths.sort();
    paths.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock TTS engine returning a 440Hz sine wave for testing.
    struct MockTtsEngine {
        sample_rate: u32,
        duration_secs: f64,
    }

    impl MockTtsEngine {
        fn new(sample_rate: u32, duration_secs: f64) -> Self {
            Self {
                sample_rate,
                duration_secs,
            }
        }
    }

    impl TtsEngine for MockTtsEngine {
        fn synthesize(&self, _text: &str) -> Result<Vec<i16>> {
            let num_samples = (self.sample_rate as f64 * self.duration_secs) as usize;
            let samples: Vec<i16> = (0..num_samples)
                .map(|i| {
                    let t = i as f64 / self.sample_rate as f64;
                    (f64::sin(2.0 * std::f64::consts::PI * 440.0 * t) * 32767.0) as i16
                })
                .collect();
            Ok(samples)
        }
    }

    #[test]
    fn mock_tts_returns_audio() {
        let engine = MockTtsEngine::new(16000, 0.5);
        let samples = engine.synthesize("Hello").unwrap();
        assert!(!samples.is_empty());
        // 16kHz * 0.5s = 8000 samples
        assert_eq!(samples.len(), 8000);
    }

    #[test]
    fn mock_tts_duration_matches() {
        let engine = MockTtsEngine::new(16000, 1.0);
        let samples = engine.synthesize("Test").unwrap();
        // 16kHz * 1.0s = 16000 samples
        assert_eq!(samples.len(), 16000);

        let engine_short = MockTtsEngine::new(16000, 0.25);
        let samples_short = engine_short.synthesize("Short").unwrap();
        // 16kHz * 0.25s = 4000 samples
        assert_eq!(samples_short.len(), 4000);
    }
}
