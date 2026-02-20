mod server;
mod transcribe;
mod tts;

use anyhow::Result;

use space_lt_common::info;
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
            // Interactive: human-friendly output
            if models.is_empty() {
                println!("No models found in {}", models_dir.display());
            } else {
                println!("Available models ({}):\n", models_dir.display());
                for (name, _) in &models {
                    println!("  space_lt_server --model {name} --language fr");
                }
            }
        } else {
            // Piped (e.g. SSH): machine-parseable name\tpath
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

    // Default: run as server (requires --model)
    let model_arg = find_arg_value(&args, "--model")
        .ok_or_else(|| anyhow::anyhow!("Usage: space_lt_server --model <name> --language <lang>\n       space_lt_server --list-models"))?;
    let model = space_lt_common::models::resolve_model_path(&model_arg);
    let language = find_arg_value(&args, "--language").unwrap_or_else(|| "en".to_string());

    // Load TTS model if --tts-model provided (stays resident for story 2-3 to wire in)
    // Note: loads before Whisper due to current code structure; G4 order will be
    // enforced when server::run() is refactored in story 2-3
    let _tts = if let Some(tts_model_dir) = find_arg_value(&args, "--tts-model") {
        let engine = tts::KokoroTts::new(std::path::Path::new(&tts_model_dir))?;
        info!("[server] TTS model loaded (not yet wired into message loop)");
        Some(engine)
    } else {
        None
    };

    server::run(&model.to_string_lossy(), &language)
}
