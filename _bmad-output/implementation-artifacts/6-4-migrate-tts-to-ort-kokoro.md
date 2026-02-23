# Story 6.4: Migrate TTS Backend to ort + Kokoro ONNX

Status: ready-for-dev

## Story

As a **user**,
I want the TTS engine to use GPU acceleration via ONNX Runtime with proper CUDA tuning,
So that voice responses are synthesized faster, reducing conversation latency.

## Context

Story 6-3 (TTS GPU Alternatives Evaluation) identified `ort` + Kokoro ONNX as the recommended migration path (score 88/100). The current sherpa-rs backend has broken CUDA support and does not expose `cudnn_conv_algo_search`, the only proven method for GPU speedup on the small Kokoro 82M model (4.6x documented in kokoro-onnx#125).

Key references:
- Evaluation document: `_bmad-output/planning-artifacts/tts-gpu-evaluation.md`
- Current TTS implementation: `server/src/tts.rs`
- TtsEngine trait: `server/src/tts.rs:8-11`

## Acceptance Criteria

1. **Given** the server with the new ort-based TTS backend
   **When** the `OrtKokoroTts` engine loads the Kokoro ONNX model
   **Then** it successfully initializes an `ort::Session` with the existing `model.onnx` file
   **And** phonemization converts input text to token IDs via espeak-ng
   **And** ONNX inference produces audio output
   **And** output is resampled from 24kHz to 16kHz mono i16 (same pipeline as current)

2. **Given** the `OrtKokoroTts` implementation
   **When** CUDA is available and the `cuda-tts` feature is enabled
   **Then** the CUDA execution provider is configured with `cudnn_conv_algo_search: DEFAULT`
   **And** inference runs on GPU
   **And** when CUDA is not available, it falls back to CPU provider gracefully

3. **Given** the `OrtKokoroTts` struct
   **When** used in the existing server architecture
   **Then** it implements `TtsEngine` trait (`synthesize(&self, text: &str) -> Result<Vec<i16>>`, `set_speed(&self, speed: f32)`)
   **And** it is `Send + Sync` (compatible with `Arc<dyn TtsEngine>`)
   **And** `synthesize()` is synchronous (no async runtime required)

4. **Given** the existing sherpa-rs backend
   **When** the migration is complete
   **Then** sherpa-rs `KokoroTts` is preserved behind a `sherpa` feature flag as fallback
   **And** `ort` backend is the new default
   **And** `--tts-backend ort|sherpa` CLI argument selects the backend

5. **Given** the complete system
   **When** performing tests
   **Then** all existing mock-based tests pass unchanged (they use `MockTtsEngine`, not the real backend)
   **And** `make check` passes
   **And** manual E2E test: conduct a voice conversation using the ort backend, verify audio quality is identical to sherpa-rs

6. **Given** the ort backend on GPU vs CPU
   **When** benchmarked on the same text input
   **Then** GPU synthesis time is logged alongside CPU baseline
   **And** results are documented in the Dev Notes of this story

## Tasks / Subtasks

- [ ] Task 1: Add ort dependency and phonemization (AC: #1)
  - [ ] 1.1: Add `ort` crate to `server/Cargo.toml` with CUDA feature flag
  - [ ] 1.2: Add espeak-ng phonemization dependency (evaluate `espeak-ng-sys` or port from kokorox/Kokoros reference code)
  - [ ] 1.3: Implement text → phonemes → token IDs pipeline (reference kokorox/Kokoros implementations)
  - [ ] 1.4: Implement `OrtKokoroTts::new()` — load `model.onnx` via `ort::Session`, load `voices.bin` for speaker embeddings, load `tokens.txt` for vocabulary

- [ ] Task 2: Implement OrtKokoroTts TtsEngine (AC: #1, #3)
  - [ ] 2.1: Implement `TtsEngine::synthesize()` — phonemize → tokenize → ONNX inference → post-process audio
  - [ ] 2.2: Reuse existing `resample_24k_to_16k()` for output conversion
  - [ ] 2.3: Implement `TtsEngine::set_speed()` — apply length_scale parameter to ONNX input
  - [ ] 2.4: Ensure `Send + Sync` via `Mutex<ort::Session>` or verify `ort::Session` is already `Send + Sync`

- [ ] Task 3: Configure CUDA execution provider (AC: #2)
  - [ ] 3.1: When `cuda-tts` feature enabled, configure `CUDAExecutionProvider` with `cudnn_conv_algo_search: DEFAULT`
  - [ ] 3.2: Implement graceful fallback to CPU if CUDA initialization fails
  - [ ] 3.3: Log execution provider used (CUDA vs CPU) at startup

- [ ] Task 4: Backend selection and feature flags (AC: #4)
  - [ ] 4.1: Restructure `server/Cargo.toml` features: `cuda-tts` now controls ort CUDA, add `sherpa` feature for legacy backend
  - [ ] 4.2: Add `--tts-backend ort|sherpa` CLI argument in `server/src/main.rs`
  - [ ] 4.3: Gate `KokoroTts` (sherpa-rs) behind `sherpa` feature flag
  - [ ] 4.4: Make `ort` the default backend when no flag specified
  - [ ] 4.5: Update `Makefile` CUDA feature flags

- [ ] Task 5: Testing and validation (AC: #5)
  - [ ] 5.1: Verify all existing tests pass unchanged (`make check`)
  - [ ] 5.2: Manual E2E test with ort backend — voice conversation, verify audio quality
  - [ ] 5.3: Manual E2E test with sherpa fallback — verify it still works behind feature flag

- [ ] Task 6: GPU benchmark and documentation (AC: #6)
  - [ ] 6.1: Add timing logs around `session.run()` to measure synthesis duration
  - [ ] 6.2: Benchmark: same text, CPU provider vs CUDA provider with `cudnn_conv_algo_search: DEFAULT`
  - [ ] 6.3: Document results in Dev Notes (latency per sentence, speedup ratio)

## Dev Notes

### Key Implementation References

**Current TtsEngine trait** (`server/src/tts.rs:8-11`):
```rust
pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str) -> Result<Vec<i16>>;
    fn set_speed(&self, speed: f32);
}
```

**Current Cargo features** (`server/Cargo.toml`):
```toml
cuda-tts = ["sherpa-rs/cuda"]  # → will change to ort CUDA
```

**ort GPU tuning** (from evaluation):
```rust
let cuda = ort::CUDAExecutionProvider::default()
    .with_conv_algorithm_search(ort::CudnnConvAlgoSearch::Default);
let session = ort::Session::builder()?
    .with_execution_providers([cuda.into()])?
    .commit_from_file("model.onnx")?;
```

**Phonemization approach:** Use espeak-ng (recommended by code review of story 6-3). It is the phonemizer Kokoro was trained with. kokorox and Kokoros both use it — reference their implementations for the text → phonemes → token IDs pipeline.

**Files to modify:**
- `server/src/tts.rs` — Add `OrtKokoroTts` struct, gate `KokoroTts` behind `sherpa` feature
- `server/Cargo.toml` — Add `ort`, espeak-ng dep, restructure features
- `server/src/main.rs` — Add `--tts-backend` CLI arg, backend selection logic
- `Makefile` — Update CUDA feature flags

**Files NOT to modify:**
- `server/src/session.rs` — Uses `Arc<dyn TtsEngine>`, backend-agnostic
- `server/src/server.rs` — Passes `Box<dyn TtsEngine>`, backend-agnostic
- All client/orchestrator/common code — no changes needed

### Previous Story Intelligence

From Story 6-3 (Evaluation):
- ort v2.0.0-rc.11: `Session` is `Send + Sync`, sync `run()` API
- Kokoro ONNX outputs f32 at 24kHz — existing resampling pipeline applies
- Key risk: phonemization reimplementation (~200-300 lines vs current ~100 lines)
- sherpa-rs can coexist behind feature flag during migration

From Story 6-2 (Streaming TTS):
- `Arc<dyn TtsEngine>` shared across threads
- Pipeline: producer thread calls `synthesize()` per sentence
- Interrupt check between sentences, not mid-synthesis
