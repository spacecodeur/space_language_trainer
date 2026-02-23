# Story 6.8: Chatterbox Turbo TTS Evaluation

Status: ready-for-dev

## Story

As a **developer**,
I want to integrate Chatterbox Turbo as an alternative TTS backend behind the existing `TtsEngine` trait and benchmark it against the current Kokoro setup,
So that I can evaluate whether its higher audio quality (Elo 2055 vs ~2000) justifies the larger model size (350M vs 82M) for the language training use case.

## Context

Chatterbox Turbo (Resemble AI) was identified in story 6-3 (TTS GPU evaluation) as the highest-quality open TTS model available:
- **Elo 2055** — highest on the TTS Arena leaderboard
- **MIT license** — fully permissive
- **Official ONNX export** — compatible with ort (which will be our ONNX runtime after story 6-4)
- **350M params** — requires GPU for real-time inference
- **23 languages** — future multi-language story (6-7) benefit
- **Voice cloning** — can clone a reference voice from a short audio clip

This story depends on **story 6-4** (migrate TTS to ort + Kokoro ONNX) being completed first, because it reuses the ort infrastructure established there.

## Acceptance Criteria

1. **Given** story 6-4 is complete (ort + Kokoro ONNX backend working)
   **When** the developer downloads the Chatterbox Turbo ONNX model files from HuggingFace
   **Then** the model files are stored locally and their structure is documented (encoder, decoder, vocoder, etc.)

2. **Given** the Chatterbox ONNX model files
   **When** the developer builds a `ChatterboxTts` struct implementing `TtsEngine`
   **Then** `synthesize()` takes a text string, runs the multi-stage ONNX pipeline (text → encoder → decoder → vocoder), and returns `Vec<i16>` at 16kHz mono
   **And** `set_speed()` is a no-op (speed control TBD for Chatterbox)
   **And** `ChatterboxTts` is `Send + Sync`

3. **Given** a working `ChatterboxTts` implementation
   **When** the developer runs a benchmark comparing Kokoro (ort) vs Chatterbox (ort) on the same hardware
   **Then** the benchmark measures: time-to-first-audio (single sentence), RTF (real-time factor), VRAM usage, and subjective quality assessment
   **And** results are documented

4. **Given** the `ChatterboxTts` implementation
   **When** the developer integrates it as a selectable backend via `--tts-backend chatterbox`
   **Then** the existing `--tts-backend` flag (from story 6-4) accepts `chatterbox` as a value
   **And** `--tts-model` points to the Chatterbox model directory
   **And** `make run-server` works with both `ort` (Kokoro) and `chatterbox` backends

5. **Given** the Chatterbox backend integrated
   **When** the developer runs a full E2E voice conversation session
   **Then** the conversation loop works end-to-end with Chatterbox TTS
   **And** streaming TTS (sentence-level synthesis) works correctly
   **And** barge-in interruption works correctly

6. **Given** all changes
   **When** `make check` is run
   **Then** all existing + new tests pass with zero warnings

## Tasks / Subtasks

- [ ] Task 1: Download and analyze Chatterbox Turbo ONNX model (AC: #1)
  - [ ] 1.1: Research the official ONNX export on HuggingFace (model files, structure, tokenizer)
  - [ ] 1.2: Download model files and document the pipeline stages (encoder, decoder, vocoder)
  - [ ] 1.3: Identify input format (text tokenization, phonemization requirements) and output format (sample rate, dtype)
  - [ ] 1.4: Determine if Chatterbox requires a different phonemizer than Kokoro's espeak-ng

- [ ] Task 2: Implement `ChatterboxTts` backend (AC: #2)
  - [ ] 2.1: Create `ChatterboxTts` struct in `server/src/tts.rs` (or a separate `server/src/tts_chatterbox.rs` if large)
  - [ ] 2.2: Load multi-stage ONNX model via ort sessions
  - [ ] 2.3: Implement text → token pipeline (tokenization, any required phonemization)
  - [ ] 2.4: Implement ONNX inference pipeline (encoder → decoder → vocoder)
  - [ ] 2.5: Add resampling if output sample rate differs from 16kHz
  - [ ] 2.6: Implement `TtsEngine` trait (`synthesize`, `set_speed` as no-op)
  - [ ] 2.7: Ensure `Send + Sync` (ort Sessions are `Send + Sync`)

- [ ] Task 3: Integrate as selectable backend (AC: #4)
  - [ ] 3.1: Add `chatterbox` option to `--tts-backend` CLI flag in `server/src/main.rs`
  - [ ] 3.2: Add model loading path for Chatterbox in server startup
  - [ ] 3.3: Update Makefile with `TTS_BACKEND=chatterbox` option if needed
  - [ ] 3.4: Add feature flag `chatterbox-tts` in `server/Cargo.toml` if additional deps needed

- [ ] Task 4: Benchmark and quality assessment (AC: #3)
  - [ ] 4.1: Benchmark time-to-first-audio for typical sentences (5-15 words)
  - [ ] 4.2: Measure RTF (synthesis time / audio duration) for both backends
  - [ ] 4.3: Measure GPU VRAM usage (Kokoro 82M vs Chatterbox 350M)
  - [ ] 4.4: Subjective quality comparison: naturalness, prosody, clarity across 10+ sample sentences
  - [ ] 4.5: Document benchmark results in evaluation report

- [ ] Task 5: E2E validation (AC: #5)
  - [ ] 5.1: Run a full voice conversation session with Chatterbox backend
  - [ ] 5.2: Verify streaming TTS (sentence-level) works correctly
  - [ ] 5.3: Verify barge-in interruption works correctly
  - [ ] 5.4: Test with 5+ minute continuous session (stability check)

- [ ] Task 6: Validation (AC: #6)
  - [ ] 6.1: Run `make check` — fmt + clippy (with `-D warnings`) + all tests pass, zero warnings
  - [ ] 6.2: Verify both Kokoro and Chatterbox backends compile and work

## Dev Notes

### CRITICAL: This Story Depends on Story 6-4

Story 6-4 establishes the ort infrastructure (dependency, CUDA provider configuration, phonemization pipeline). This story builds on top of that. Do NOT start this before 6-4 is done.

### CRITICAL: Multi-Stage ONNX Pipeline

Unlike Kokoro (single ONNX model), Chatterbox Turbo likely has multiple ONNX files (encoder, decoder, vocoder). The inference pipeline must:
1. Tokenize input text
2. Run encoder (text → latent representation)
3. Run decoder (latent → mel spectrogram)
4. Run vocoder (mel → raw audio waveform)

Each stage is a separate `ort::Session`. All sessions must be loaded at startup and shared via `Arc`.

### CRITICAL: 350M Params — GPU Required

Chatterbox at 350M params will NOT run at real-time on CPU. GPU inference via ort CUDA provider is mandatory. This means:
- The `--tts-backend chatterbox` option requires a CUDA-capable GPU
- VRAM budget must be assessed: Whisper + Chatterbox must fit in available VRAM
- If VRAM is tight, consider whether Whisper should move to CPU when using Chatterbox

### Model Source

Official ONNX export: check `resemble-ai/chatterbox` GitHub and HuggingFace for ONNX artifacts. If no official ONNX exists, check community exports or use `optimum` Python tool to convert.

### Audio Output

Chatterbox output format (sample rate, dtype) must be verified during Task 1. If different from 24kHz (Kokoro), the resampling pipeline in `TtsEngine` wrapper needs adjustment.

### Voice Cloning (Out of Scope)

Chatterbox supports voice cloning from a reference audio clip. This is out of scope for this story — use the default voice. Voice cloning can be explored in a future story.

### Speed Control (Out of Scope)

Chatterbox may not support speed control natively. `set_speed()` will be a no-op. Speed control for Chatterbox can be explored separately if needed.

### References

- [Story 6-3 evaluation](/_bmad-output/planning-artifacts/tts-gpu-evaluation.md) — Chatterbox scored 74/100 (Rank #2)
- [Chatterbox GitHub](https://github.com/resemble-ai/chatterbox)
- [TTS Arena Leaderboard](https://huggingface.co/spaces/Pendrokar/TTS-Spaces-Arena) — Elo 2055
- [Story 6-4](/_bmad-output/implementation-artifacts/6-4-migrate-tts-to-ort-kokoro.md) — ort infrastructure dependency
