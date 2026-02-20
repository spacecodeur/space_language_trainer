mod listener;
mod server;
mod session;
mod transcribe;
mod tts;

use anyhow::Result;

use space_lt_common::{debug, info};
use transcribe::Transcriber;
use tts::TtsEngine;

fn find_arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Parse --debug flag
    if args.iter().any(|a| a == "--debug") {
        space_lt_common::log::set_debug(true);
    }

    // --list-models: print local models and exit
    if args.iter().any(|a| a == "--list-models") {
        use std::io::IsTerminal;
        let models_dir = space_lt_common::models::default_models_dir();
        let models = space_lt_common::models::scan_models(&models_dir)?;
        if std::io::stdout().is_terminal() {
            if models.is_empty() {
                println!("No models found in {}", models_dir.display());
            } else {
                println!("Available models ({}):\n", models_dir.display());
                for (name, _) in &models {
                    println!("  space_lt_server --model {name} --language fr");
                }
            }
        } else {
            for (name, path) in &models {
                println!("{name}\t{}", path.display());
            }
        }
        return Ok(());
    }

    // --tts-test: synthesize text, write WAV, exit (requires --tts-model)
    if let Some(test_text) = find_arg_value(&args, "--tts-test") {
        let tts_model_dir = find_arg_value(&args, "--tts-model")
            .ok_or_else(|| anyhow::anyhow!("--tts-test requires --tts-model <path>"))?;
        let tts = tts::KokoroTts::new(std::path::Path::new(&tts_model_dir))?;
        let samples = tts.synthesize(&test_text)?;
        info!(
            "[server] Synthesized {} samples ({:.2}s at 16kHz)",
            samples.len(),
            samples.len() as f64 / 16000.0
        );
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let output_path = "tts_test_output.wav";
        let mut writer = hound::WavWriter::create(output_path, spec)?;
        for &sample in &samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        info!("[server] WAV written to {output_path}");
        return Ok(());
    }

    // Default: run as daemon server (requires --model and --tts-model)
    let model_arg = find_arg_value(&args, "--model").ok_or_else(|| {
        anyhow::anyhow!(
            "Usage: space_lt_server --model <name> --tts-model <path> [--port <port>] [--socket-path <path>]\n       space_lt_server --list-models\n       space_lt_server --tts-test \"text\" --tts-model <path>"
        )
    })?;
    let model = space_lt_common::models::resolve_model_path(&model_arg);
    let language = find_arg_value(&args, "--language").unwrap_or_else(|| "en".to_string());

    let tts_model_dir = find_arg_value(&args, "--tts-model").ok_or_else(|| {
        anyhow::anyhow!("Daemon mode requires --tts-model <path> (Kokoro model directory)")
    })?;

    let port: u16 = find_arg_value(&args, "--port")
        .map(|p| p.parse())
        .transpose()
        .map_err(|e| anyhow::anyhow!("Invalid --port value: {e}"))?
        .unwrap_or(9500);

    let socket_path = find_arg_value(&args, "--socket-path")
        .unwrap_or_else(|| "/tmp/space_lt_server.sock".to_string());

    // Sequential model loading (G4): Whisper first, then Kokoro
    info!("[server] Loading Whisper model: {model_arg}...");
    let start = std::time::Instant::now();
    let mut transcriber = transcribe::LocalTranscriber::new(&model.to_string_lossy(), &language)?;
    info!(
        "[server] Whisper model loaded in {:.1}s",
        start.elapsed().as_secs_f64()
    );

    // Warm up Whisper (GPU graph init)
    debug!("[server] Warming up Whisper...");
    let silence = vec![0i16; 16000];
    let _ = transcriber.transcribe(&silence);
    debug!("[server] Whisper warm-up complete");

    info!("[server] Loading TTS model: {tts_model_dir}...");
    let start = std::time::Instant::now();
    let tts_engine = tts::KokoroTts::new(std::path::Path::new(&tts_model_dir))?;
    info!(
        "[server] TTS model loaded in {:.1}s",
        start.elapsed().as_secs_f64()
    );

    // Run daemon
    server::run_daemon(
        Box::new(transcriber),
        Box::new(tts_engine),
        port,
        std::path::Path::new(&socket_path),
    )
}
