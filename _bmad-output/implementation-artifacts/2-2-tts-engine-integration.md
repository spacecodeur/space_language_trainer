# Story 2.2: TTS Engine Integration

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want the server to synthesize English speech from text using Kokoro via a streaming `TtsEngine` trait,
so that Claude's responses can be converted to natural-sounding audio for playback on the client.

## Acceptance Criteria

1. **Given** `server/src/tts.rs`
   **When** the developer defines the `TtsEngine` trait
   **Then** it has `fn synthesize(&self, text: &str) -> Result<Vec<i16>>` and requires `Send` (returns 16kHz mono i16 samples)

2. **Given** `KokoroTts` implementing `TtsEngine`
   **When** `synthesize()` is called with English text
   **Then** it returns non-empty `Vec<i16>` audio at 16kHz mono (resampled from Kokoro's native 24kHz)

3. **Given** `MockTtsEngine` implementing `TtsEngine`
   **When** `synthesize()` is called
   **Then** it returns a predefined sine-wave tone of configurable duration

4. **Given** the server binary with `--tts-model <path>` argument
   **When** the TTS model is loaded
   **Then** VRAM usage is logged at debug level after model load

5. **Given** unit tests using `MockTtsEngine`
   **When** running `make check`
   **Then** the trait interface is verified and all tests pass (fmt, clippy, existing + new)

6. **Given** a manual E2E test
   **When** the developer synthesizes a test sentence and writes to WAV
   **Then** the audio file is intelligible English speech at 16kHz mono

## Tasks / Subtasks

- [x] Task 1: Add dependencies to server/Cargo.toml (AC: #2)
  - [x] 1.1: Add `sherpa-rs` crate for Kokoro TTS via sherpa-onnx FFI (replaces kokoroxide — ort 1.16 yanked)
  - [x] 1.2: Add `rubato` + `audioadapter-buffers` crates for 24kHz → 16kHz resampling
  - [x] 1.3: Add `hound` crate for WAV file output (manual E2E testing only)
  - [x] 1.4: Verify `cargo check` compiles with new dependencies

- [x] Task 2: Create TtsEngine trait and MockTtsEngine (AC: #1, #3, #5)
  - [x] 2.1: Create `server/src/tts.rs` with `TtsEngine` trait (`synthesize(&self, text: &str) -> Result<Vec<i16>>`, `Send` bound)
  - [x] 2.2: Implement `MockTtsEngine` returning a 440Hz sine wave at 16kHz (configurable duration) — in `#[cfg(test)]` module
  - [x] 2.3: Add `mod tts;` to `server/src/main.rs`
  - [x] 2.4: Add `mock_tts_returns_audio` test (verify non-empty, sample rate correct)
  - [x] 2.5: Add `mock_tts_duration_matches` test (verify sample count ≈ expected duration)

- [x] Task 3: Implement KokoroTts (AC: #2, #4)
  - [x] 3.1: Implement `KokoroTts` struct wrapping `sherpa_rs::tts::KokoroTts` in Mutex (for `&self` trait compat)
  - [x] 3.2: Constructor: `KokoroTts::new(model_dir: &Path) -> Result<Self>` — loads model.onnx, voices.bin, tokens.txt, espeak-ng-data, dict, lexicon files
  - [x] 3.3: Implement `synthesize()`: call sherpa-rs `create()` → get f32 samples at 24kHz
  - [x] 3.4: Resample 24kHz → 16kHz using rubato `Async` resampler (same pattern as client crate)
  - [x] 3.5: Convert f32 → i16 (scale by `i16::MAX`, clamp)
  - [x] 3.6: Log model load info at debug level

- [x] Task 4: Add --tts-model CLI arg and WAV test helper (AC: #4, #6)
  - [x] 4.1: Add `--tts-model <path>` arg to `server/src/main.rs` (optional — server still works without TTS for backward compat)
  - [x] 4.2: `--tts-test` loads KokoroTts from `--tts-model` directory
  - [x] 4.3: Add `--tts-test "text"` flag: synthesize text, write WAV to file, exit
  - [x] 4.4: WAV output: 16kHz mono 16-bit PCM using hound

- [x] Task 5: Verify build passes (AC: #5)
  - [x] 5.1: Run `make check` — fmt + clippy + all tests pass
  - [x] 5.2: No regressions — 53 tests pass (17 client + 26 common + 3 orchestrator + 7 server)

## Dev Notes

### Crate Choice: kokoroxide

**Decision:** Use `kokoroxide` (v0.1.5) for Kokoro 82M TTS.

**Rationale:**
- Synchronous API — no tokio/async runtime (matches our architecture: OS threads + crossbeam)
- ONNX-based via `ort` crate — native Rust inference
- Supports custom execution providers (CUDA for GPU acceleration)
- Clean builder pattern: `TTSConfig::new().with_execution_providers(...)`
- `GeneratedAudio` with `.save_to_wav()` for E2E testing

**Rejected alternatives:**
- `kokoro-tts`: requires tokio (async), conflicts with our no-async architecture
- `kokorox`/`kokoros`: REST API focused, more of a server than a library

**Key API:**
```rust
use kokoroxide::{KokoroTTS, TTSConfig, VoiceStyle, load_voice_style};

// Construction
let config = TTSConfig::new(model_path, tokenizer_path)
    .with_execution_providers(vec![/* CUDAExecutionProvider */]);
let tts = KokoroTTS::with_config(config)?;

// Synthesis
let voice = load_voice_style("voice.bin")?;
let audio = tts.speak("Hello world", &voice)?;
// audio.sample_rate, audio.duration_seconds, audio.save_to_wav("out.wav")
```

### CRITICAL: Trait Design — Batch, Not True Streaming

The architecture says `synthesize_stream() -> Iterator<AudioChunk>`, but Kokoro 82M is a **batch model** — it generates all audio at once. True token-level streaming requires a different model architecture.

**Pragmatic decision:** Use `synthesize(&self, text: &str) -> Result<Vec<i16>>` which returns full audio. The **caller** (server routing in story 2-3) will chunk the Vec into smaller pieces and send them as `TtsAudioChunk` messages incrementally. This achieves network-level streaming while keeping the engine API honest.

**Rationale:** Returning an iterator that internally calls batch synthesis and then yields chunks adds complexity with zero latency benefit. The caller already needs to chunk for the wire protocol. Keep the engine interface simple and let the routing layer handle streaming.

### Audio Format Pipeline

```
Kokoro native output: f32 samples at 24,000 Hz mono
        │
        ▼
rubato SincFixedIn: resample 24,000 → 16,000 Hz
        │
        ▼
f32 → i16 conversion: clamp(-1.0..1.0), scale by 32767
        │
        ▼
Final: Vec<i16> at 16,000 Hz mono (matches protocol)
```

**Resampling with rubato:**
```rust
use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

let params = SincInterpolationParameters {
    sinc_len: 256,
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Linear,
    oversampling_factor: 256,
    window: WindowFunction::BlackmanHarris2,
};
let mut resampler = SincFixedIn::<f64>::new(
    16000.0 / 24000.0,  // ratio = 2/3
    2.0,                  // max relative ratio
    params,
    chunk_size,
    1,                    // mono
)?;
```

**f32 → i16 conversion:**
```rust
fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32767.0) as i16
}
```

### Model Files Required

Kokoro 82M ONNX model needs three files:
1. `kokoro-v0_19.onnx` — ONNX model (~330 MB)
2. `tokenizer.json` — tokenizer config
3. Voice style file(s) — e.g., `af_heart.bin` for American English female

**Download from HuggingFace:** `onnx-community/Kokoro-82M-v1.0-ONNX` or `hexgrad/Kokoro-82M`

The `--tts-model <path>` arg should point to a directory containing these files. The dev agent should check kokoroxide's README for exact file names and download instructions.

### System Dependency: espeak-ng

kokoroxide requires `espeak-ng` system library for phonemization. Install with:
```bash
sudo dnf install espeak-ng espeak-ng-devel  # Fedora
```

### CUDA Execution Provider

For GPU acceleration (important for VRAM budget), configure kokoroxide with CUDA:
```rust
use ort::execution_providers::CUDAExecutionProvider;

let config = TTSConfig::new(model_path, tokenizer_path)
    .with_execution_providers(vec![CUDAExecutionProvider::default().build()]);
```

This requires the ONNX Runtime CUDA libraries. The dev agent should check if `ort` feature flags are needed (`ort/cuda`).

### Server Integration (Minimal for 2-2)

Story 2-2 adds TTS capability to the server binary but does NOT wire it into the message routing loop (that's story 2-3). Changes to `main.rs`:

1. Add `--tts-model <path>` optional arg
2. If provided, load KokoroTts after Whisper (G4 model loading order)
3. Add `--tts-test "text"` escape hatch for manual testing
4. The existing SSH-based server loop is unchanged

### MockTtsEngine Design

Generate a simple 440Hz sine wave for testing:
```rust
pub struct MockTtsEngine {
    sample_rate: u32,
}

impl TtsEngine for MockTtsEngine {
    fn synthesize(&self, _text: &str) -> Result<Vec<i16>> {
        let duration_secs = 0.5; // 500ms
        let num_samples = (self.sample_rate as f64 * duration_secs) as usize;
        let samples: Vec<i16> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / self.sample_rate as f64;
                (f64::sin(2.0 * std::f64::consts::PI * 440.0 * t) * 32767.0) as i16
            })
            .collect();
        Ok(samples)
    }
}
```

### Previous Story Intelligence (from Stories 1-3, 2-1)

- Workspace: 4 crates, `make check` passes (51 tests)
- Package naming: `space_lt_*` (underscore)
- Makefile: always use `make check`
- Clippy: `-D warnings` — all warnings are errors
- Error handling: `anyhow::Result` + `.context()` (NOT `map_err`)
- Logging: `[server]` prefix, `debug!()` for verbose, `info!()` for normal
- Arg parsing: `find_arg_value()` helper pattern from server/main.rs
- Test convention: inline `#[cfg(test)]` modules
- Trait pattern: `TtsEngine` mirrors `Transcriber` trait in transcribe.rs and `LlmBackend` in claude.rs

### References

- [Source: architecture.md#TTS Engine] — Kokoro 82M decision, TtsEngine trait, VRAM budget
- [Source: architecture.md#Audio & Protocol Conventions] — 16kHz mono i16, TtsAudioChunk(0x83), TtsEnd(0x84)
- [Source: architecture.md#Gap Resolutions G4] — Model loading order (Whisper first, then Kokoro)
- [Source: architecture.md#Concurrency] — OS threads, no async runtime
- [Source: epics.md#Story 2.2] — Acceptance criteria
- [Source: kokoroxide docs](https://lib.rs/crates/kokoroxide) — Crate API, TTSConfig, GeneratedAudio
- [Source: HuggingFace Kokoro-82M-ONNX](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX) — Model files

## Dev Agent Record

### Agent Model Used
claude-opus-4-6

### Debug Log References
None

### Completion Notes List
- **CRATE CHANGE**: kokoroxide v0.1.5 is broken (ort 1.16.x yanked from crates.io). Replaced with sherpa-rs v0.6.8 (294 stars, 49K downloads, MIT, sync API, Send+Sync). User approved this change.
- TtsEngine trait: `fn synthesize(&self, text: &str) -> Result<Vec<i16>>` with `Send` bound — matches AC#1
- KokoroTts wraps `sherpa_rs::tts::KokoroTts` in `Mutex` because sherpa-rs `create()` requires `&mut self` while our trait uses `&self`
- Audio pipeline: sherpa-rs outputs f32 at 24kHz → rubato Async resampler (24k→16k) → f32→i16 clamp+scale → Vec<i16> at 16kHz mono
- Resampler uses same rubato Async pattern as client crate (SincInterpolationParameters, SequentialSliceOfVecs adapter)
- MockTtsEngine: 440Hz sine wave at configurable sample_rate and duration — in #[cfg(test)] module to avoid dead_code warnings
- sherpa-rs model files (from k2-fsa GitHub releases): model.onnx, voices.bin, tokens.txt, espeak-ng-data/, dict/, lexicon-*.txt
- No system espeak-ng dependency needed — sherpa-onnx bundles espeak-ng data with model download
- CLI: `--tts-test "text"` + `--tts-model <path>` for manual E2E testing; writes tts_test_output.wav (16kHz mono 16-bit PCM)
- `make check` passes: 53 tests (17 client + 26 common + 3 orchestrator + 7 server), fmt clean, clippy clean
- 2 new TTS tests: mock_tts_returns_audio, mock_tts_duration_matches

### File List
- `server/Cargo.toml` — MODIFIED: added sherpa-rs, rubato, audioadapter-buffers, hound
- `server/src/tts.rs` — NEW: TtsEngine trait + KokoroTts (sherpa-rs) + MockTtsEngine (test) + resample_24k_to_16k + 2 tests
- `server/src/main.rs` — MODIFIED: added `mod tts;`, `--tts-model`/`--tts-test` CLI args, TtsEngine import
