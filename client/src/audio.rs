use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::Sender;
use rubato::Resampler;

use space_lt_common::warn;

pub struct CaptureConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Maximum number of attempts to start the audio capture stream.
const CAPTURE_MAX_ATTEMPTS: u32 = 3;
/// Delay between capture stream retry attempts.
const CAPTURE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

pub fn start_capture(
    device: &cpal::Device,
    sender: Sender<Vec<i16>>,
) -> Result<(cpal::Stream, CaptureConfig)> {
    let config = device
        .default_input_config()
        .context("Failed to get default input config")?;

    let sample_rate = config.sample_rate();
    let channels = config.channels();

    let stream_config: cpal::StreamConfig = config.into();

    for attempt in 1..=CAPTURE_MAX_ATTEMPTS {
        let sender_clone = sender.clone();

        let build_result = device
            .build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let _ = sender_clone.try_send(data.to_vec());
                },
                |err: cpal::StreamError| {
                    warn!("[client] Audio stream error: {err}");
                },
                None,
            )
            .context("building input stream");

        match build_result {
            Ok(stream) => match stream.play() {
                Ok(()) => {
                    return Ok((
                        stream,
                        CaptureConfig {
                            sample_rate,
                            channels,
                        },
                    ));
                }
                Err(e) => {
                    let err = anyhow::anyhow!("starting audio stream: {e}");
                    if attempt < CAPTURE_MAX_ATTEMPTS {
                        warn!(
                            "[client] Audio capture attempt {attempt}/{CAPTURE_MAX_ATTEMPTS} failed: {err}, retrying..."
                        );
                        std::thread::sleep(CAPTURE_RETRY_DELAY);
                    } else {
                        return Err(err);
                    }
                }
            },
            Err(e) => {
                if attempt < CAPTURE_MAX_ATTEMPTS {
                    warn!(
                        "[client] Audio capture attempt {attempt}/{CAPTURE_MAX_ATTEMPTS} failed: {e}, retrying..."
                    );
                    std::thread::sleep(CAPTURE_RETRY_DELAY);
                } else {
                    return Err(e);
                }
            }
        }
    }

    unreachable!("loop always returns or errors")
}

/// Resampler function type. Accepts audio samples and returns resampled output.
///
/// **Flush convention:** Calling with an empty slice (`&[]`) flushes the internal
/// carry-over buffer, processing any remaining samples with zero-padding. This
/// MUST be called when a TTS stream ends (on `TtsEnd`) to avoid losing trailing
/// audio. Do NOT call with an empty slice mid-stream — it will corrupt the
/// resampler state for subsequent chunks.
pub type ResamplerFn = Box<dyn FnMut(&[i16]) -> Vec<i16>>;

/// Create a resampler that converts audio from `source_rate` to `target_rate`.
///
/// Uses a carry-over buffer to avoid zero-padding artifacts at chunk boundaries.
/// Only full resampler frames (1024 samples) are processed; leftover samples are
/// carried over to the next call. Call with `&[]` to flush remaining samples at
/// end of stream.
pub fn create_resampler(source_rate: u32, target_rate: u32, channels: u16) -> Result<ResamplerFn> {
    if source_rate == target_rate && channels == 1 {
        return Ok(Box::new(|samples: &[i16]| samples.to_vec()));
    }

    let ch = channels as usize;
    let ratio = target_rate as f64 / source_rate as f64;

    use rubato::{
        Async, FixedAsync, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Quadratic,
        oversampling_factor: 256,
        window: WindowFunction::Blackman2,
    };

    let chunk_size = 1024;
    let mut resampler =
        Async::<f64>::new_sinc(ratio, 1.1, &params, chunk_size, 1, FixedAsync::Input)
            .map_err(|e| anyhow::anyhow!("Failed to create resampler: {e}"))?;

    // Carry-over buffer: leftover samples from previous call (< chunk_size)
    let mut leftover: Vec<f64> = Vec::new();

    Ok(Box::new(move |samples: &[i16]| {
        use audioadapter_buffers::direct::SequentialSliceOfVecs;

        let is_flush = samples.is_empty();

        // Convert to mono f64 normalized [-1.0, 1.0]
        let mono: Vec<f64> = if ch == 1 {
            samples.iter().map(|&s| s as f64 / 32768.0).collect()
        } else {
            samples
                .chunks(ch)
                .map(|frame| {
                    let sum: f64 = frame.iter().map(|&s| s as f64).sum();
                    (sum / ch as f64) / 32768.0
                })
                .collect()
        };

        // Prepend carry-over from previous call
        let mut combined = std::mem::take(&mut leftover);
        combined.extend_from_slice(&mono);

        // If flushing and nothing to process, return empty
        if combined.is_empty() {
            return Vec::new();
        }

        let mut output_all: Vec<i16> = Vec::new();
        let mut offset = 0;

        // Process only full chunk_size frames (no zero-padding during streaming)
        while offset + chunk_size <= combined.len() {
            let chunk = &combined[offset..offset + chunk_size];
            let input_data: Vec<Vec<f64>> = vec![chunk.to_vec()];
            let adapter = match SequentialSliceOfVecs::new(&input_data, 1, chunk_size) {
                Ok(a) => a,
                Err(e) => {
                    warn!("Resample adapter error: {e}");
                    leftover.clear();
                    return output_all;
                }
            };

            match resampler.process(&adapter, 0, None) {
                Ok(output) => {
                    for &s in output.take_data().iter() {
                        output_all.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }
                }
                Err(e) => {
                    warn!("Resample error: {e}");
                    leftover.clear();
                    return output_all;
                }
            }
            offset += chunk_size;
        }

        let remainder = &combined[offset..];

        if is_flush && !remainder.is_empty() {
            // Flush: zero-pad the final partial chunk (only acceptable at stream end)
            let mut padded = remainder.to_vec();
            padded.resize(chunk_size, 0.0);
            let input_data: Vec<Vec<f64>> = vec![padded];
            let adapter = match SequentialSliceOfVecs::new(&input_data, 1, chunk_size) {
                Ok(a) => a,
                Err(e) => {
                    warn!("Resample adapter error on flush: {e}");
                    return output_all;
                }
            };
            match resampler.process(&adapter, 0, None) {
                Ok(output) => {
                    let data = output.take_data();
                    let expected = (remainder.len() as f64 * ratio).ceil() as usize;
                    for &s in &data[..expected.min(data.len())] {
                        output_all.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }
                }
                Err(e) => {
                    warn!("Resample error on flush: {e}");
                }
            }
            // leftover stays empty after flush
        } else if !is_flush {
            // Carry over remaining samples to next call
            leftover = remainder.to_vec();
        }

        output_all
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_noop_mono() {
        let mut resample = create_resampler(16000, 16000, 1).unwrap();
        let input: Vec<i16> = (0..1600).collect();
        let output = resample(&input);
        assert_eq!(output, input);
    }

    #[test]
    fn resampler_48k_to_16k() {
        let mut resample = create_resampler(48000, 16000, 1).unwrap();
        // 100ms at 48kHz = 4800 samples
        let input: Vec<i16> = vec![0; 4800];
        let output = resample(&input);
        let flush = resample(&[]);
        let total = output.len() + flush.len();
        // Expected ~1600 samples (100ms at 16kHz), allow some margin
        let expected = 1600;
        let margin = 200;
        assert!(
            (total as i32 - expected as i32).unsigned_abs() < margin,
            "Expected ~{expected} samples, got {total}",
        );
    }

    /// Generate a 440Hz sine wave at the given sample rate.
    fn sine_wave(sample_rate: u32, duration_secs: f64) -> Vec<i16> {
        let num_samples = (sample_rate as f64 * duration_secs) as usize;
        (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (f64::sin(2.0 * std::f64::consts::PI * 440.0 * t) * 20000.0) as i16
            })
            .collect()
    }

    #[test]
    fn resampler_carry_over_no_discontinuity() {
        let mut resample = create_resampler(16000, 48000, 1).unwrap();
        let signal = sine_wave(16000, 2.0); // 32000 samples
        let chunk_size = 4000;

        // Process in chunks (simulating TCP chunks)
        let mut chunked_output: Vec<i16> = Vec::new();
        for chunk in signal.chunks(chunk_size) {
            chunked_output.extend_from_slice(&resample(chunk));
        }
        chunked_output.extend_from_slice(&resample(&[])); // flush

        // Verify no discontinuity: max sample-to-sample delta should be bounded
        // For a 440Hz sine at 48kHz, max natural delta ≈ 2π*440/48000 * 20000 ≈ 1152
        // We use 3000 as threshold to catch pops while allowing natural signal variation
        let mut max_delta: i32 = 0;
        for w in chunked_output.windows(2) {
            let delta = (w[1] as i32 - w[0] as i32).abs();
            max_delta = max_delta.max(delta);
        }
        assert!(
            max_delta < 3000,
            "Discontinuity detected: max sample delta = {max_delta} (threshold 3000)"
        );
    }

    #[test]
    fn resampler_flush_produces_remaining_samples() {
        let mut resample = create_resampler(16000, 48000, 1).unwrap();
        // 500 samples < chunk_size (1024), all goes to carry-over
        let input = sine_wave(16000, 0.03125); // 500 samples
        let output = resample(&input);
        let flush = resample(&[]);
        let total = output.len() + flush.len();
        // Expected ~1500 samples (500 * 3.0 ratio)
        assert!(
            total > 0,
            "Flush should produce output for carried-over samples"
        );
        let expected = 1500;
        assert!(
            (total as i32 - expected as i32).unsigned_abs() < 100,
            "Expected ~{expected} samples, got {total}"
        );
    }

    #[test]
    fn resampler_carry_over_matches_single_pass() {
        // Chunked processing (4000+4000 + flush)
        let mut chunked = create_resampler(16000, 48000, 1).unwrap();
        let signal = sine_wave(16000, 0.5); // 8000 samples
        let mut chunked_out: Vec<i16> = Vec::new();
        chunked_out.extend_from_slice(&chunked(&signal[..4000]));
        chunked_out.extend_from_slice(&chunked(&signal[4000..]));
        chunked_out.extend_from_slice(&chunked(&[])); // flush

        // Single-pass processing (8000 + flush)
        let mut single = create_resampler(16000, 48000, 1).unwrap();
        let mut single_out: Vec<i16> = Vec::new();
        single_out.extend_from_slice(&single(&signal));
        single_out.extend_from_slice(&single(&[])); // flush

        // Output lengths should match within resampler chunk_size (1024 output frames)
        let margin = 1024;
        let diff = (chunked_out.len() as i32 - single_out.len() as i32).unsigned_abs();
        assert!(
            diff < margin as u32,
            "Chunked ({}) vs single-pass ({}) differ by {diff} (max {margin})",
            chunked_out.len(),
            single_out.len()
        );
    }

    #[test]
    fn resampler_noop_flush_is_empty() {
        let mut resample = create_resampler(16000, 16000, 1).unwrap();
        let input: Vec<i16> = (0..100).collect();
        let output = resample(&input);
        assert_eq!(output, input);
        // Flush on no-op resampler should return empty (no carry-over)
        let flush = resample(&[]);
        assert!(flush.is_empty() || flush == Vec::<i16>::new());
    }
}
