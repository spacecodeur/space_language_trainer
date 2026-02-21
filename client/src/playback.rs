use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Receiver;

use space_lt_common::{info, warn};

/// Start an audio output stream that plays TTS audio from the given channel.
///
/// Returns the cpal Stream (must be kept alive for playback to continue)
/// and the actual output sample rate (for resampling if needed).
pub fn start_playback(audio_rx: Receiver<Vec<i16>>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No default audio output device found"))?;

    let device_name = device
        .description()
        .map(|d: cpal::DeviceDescription| d.name().to_string())
        .unwrap_or_else(|_| "Default".into());

    let default_config = device
        .default_output_config()
        .context("getting default output config")?;

    let native_rate = default_config.sample_rate();
    info!("[client] Playback device: {device_name}, native rate: {native_rate}Hz");

    // Use 16kHz if supported, otherwise use native rate (caller will resample)
    let output_rate = if native_rate == 16000 {
        16000
    } else {
        native_rate
    };

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: output_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    // Residual samples from a chunk that didn't fully fit into the previous callback.
    // Capped at 1 second of audio (output_rate samples) to prevent unbounded growth.
    let max_leftover = output_rate as usize;
    let mut leftover: Vec<i16> = Vec::new();

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let mut offset = 0;

                // First, drain any leftover samples from the previous callback
                if !leftover.is_empty() {
                    let n = leftover.len().min(data.len());
                    data[..n].copy_from_slice(&leftover[..n]);
                    offset = n;
                    if n < leftover.len() {
                        leftover.drain(..n);
                    } else {
                        leftover.clear();
                    }
                }

                // Then pull from channel
                while offset < data.len() {
                    match audio_rx.try_recv() {
                        Ok(chunk) => {
                            let remaining = data.len() - offset;
                            let n = chunk.len().min(remaining);
                            data[offset..offset + n].copy_from_slice(&chunk[..n]);
                            offset += n;
                            // Save leftover if chunk was bigger than remaining space
                            if n < chunk.len() && leftover.len() < max_leftover {
                                let cap = (max_leftover - leftover.len()).min(chunk.len() - n);
                                leftover.extend_from_slice(&chunk[n..n + cap]);
                            }
                        }
                        Err(_) => {
                            // No data available — fill remainder with silence (self-healing)
                            if offset > 0 {
                                // Buffer underrun: started writing audio but ran out mid-callback
                                space_lt_common::debug!(
                                    "[client] Playback buffer underrun, filling with silence"
                                );
                            }
                            data[offset..].fill(0);
                            break;
                        }
                    }
                }
            },
            |err| warn!("[client] Playback error: {err}"),
            None,
        )
        .context("building output stream")?;

    stream.play().context("starting playback stream")?;

    Ok((stream, output_rate))
}
