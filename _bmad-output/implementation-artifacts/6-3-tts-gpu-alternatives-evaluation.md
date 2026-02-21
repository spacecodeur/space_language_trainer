# Story 6.3: TTS GPU Alternatives Evaluation

Status: review

## Story

As a **developer**,
I want to evaluate alternative TTS backends that support GPU acceleration and compare them against the current sherpa-rs/Kokoro setup,
So that I can make an informed decision on which alternative to implement in the next story.

## Acceptance Criteria

1. **Given** the current TTS setup (sherpa-rs v0.6.8 + Kokoro 82M, CPU-only stable)
   **When** the developer researches available Rust TTS alternatives
   **Then** at least 3 alternatives are identified and cataloged with: crate name, version, underlying engine, GPU backends supported, last release date, GitHub stars/activity

2. **Given** the list of alternatives
   **When** each is evaluated against defined criteria
   **Then** a comparison matrix is produced covering: GPU support (CUDA, TensorRT, DirectML), maturity (version stability, issue count, release frequency), API compatibility with `TtsEngine` trait, Kokoro model support, audio output format, thread safety (`Send + Sync`), sync vs async API, build complexity (native deps, CUDA toolkit version)

3. **Given** the current codebase integration surface
   **When** the developer assesses migration effort for each alternative
   **Then** the assessment identifies: files to modify, trait compatibility (can it implement `TtsEngine`?), resampling needs (native sample rate vs 16kHz), model file structure differences, feature flag changes, Makefile/build changes, test mock impact

4. **Given** all evaluation data
   **When** the developer produces a final recommendation
   **Then** the recommendation includes: ranked alternatives with rationale, recommended candidate for the next story (implementation), identified risks and blockers, estimated migration effort (low/medium/high per component)

5. **Given** the evaluation is complete
   **When** deliverables are verified
   **Then** the evaluation document is saved as a planning artifact
   **And** no code changes are made (research-only story)
   **And** `make check` still passes (no regressions from research)

## Tasks / Subtasks

- [x] Task 1: Catalog alternatives (AC: #1)
  - [x] 1.1: Research `ort` crate (ONNX Runtime wrapper) — GPU providers, Kokoro ONNX model compatibility, API surface
  - [x] 1.2: Research `kokorox` crate — maturity, GPU support, API design, dependencies
  - [x] 1.3: Research `Kokoros` crate — maturity, GPU support, API design
  - [x] 1.4: Research any other Rust TTS crates with GPU support (candle-based, tch-rs, etc.)
  - [x] 1.5: Check if sherpa-rs has open PRs or forks exposing CUDA provider options
  - [x] 1.6: Compile catalog table with: name, version, engine, GPU backends, activity metrics

- [x] Task 2: Define evaluation criteria and build comparison matrix (AC: #2)
  - [x] 2.1: Define weighted criteria (GPU perf, maturity, API fit, build complexity, maintenance risk)
  - [x] 2.2: Evaluate each alternative against criteria
  - [x] 2.3: Run quick feasibility checks where possible (e.g., `cargo add` + compile test in isolated branch)
  - [x] 2.4: Produce comparison matrix table

- [x] Task 3: Assess migration effort per alternative (AC: #3)
  - [x] 3.1: Map current `TtsEngine` trait surface (`synthesize`, `set_speed`, `Send + Sync`)
  - [x] 3.2: For each alternative: can it implement `TtsEngine` directly? What adapter is needed?
  - [x] 3.3: Identify resampling requirements (native sample rate → 16kHz i16)
  - [x] 3.4: Identify model file structure differences (ONNX model path, voice files, phoneme data)
  - [x] 3.5: Assess build system impact (Cargo features, native deps, CUDA toolkit version, CI)
  - [x] 3.6: Produce migration effort summary per alternative (low/medium/high per component)

- [x] Task 4: Produce recommendation (AC: #4)
  - [x] 4.1: Rank alternatives based on weighted criteria
  - [x] 4.2: Identify the recommended candidate for story 6-4 (implementation)
  - [x] 4.3: Document risks, blockers, and open questions
  - [x] 4.4: Write the evaluation document and save as planning artifact

- [x] Task 5: Verify no regressions (AC: #5)
  - [x] 5.1: Run `make check` — must pass unchanged
  - [x] 5.2: Verify no code changes were committed

## Dev Notes

### CRITICAL: This Is a Research-Only Story

**NO code changes.** The deliverable is an evaluation document saved to `_bmad-output/planning-artifacts/tts-gpu-evaluation.md`. The next story (6-4) will implement the chosen alternative.

### CRITICAL: Why GPU TTS Matters (and Why It Doesn't)

**Current state:** Kokoro 82M on CPU via sherpa-rs takes ~0.3-0.5s per sentence. With streaming pipeline (story 6-2), time-to-first-audio is acceptable.

**Problem:** sherpa-rs CUDA crashes (known issue, disabled by default). No access to CUDA provider options (`cudnn_conv_algo_search`) that provide the only proven speedup for small models like Kokoro.

**Paradox:** Kokoro is so small (82M params) that naive GPU offload is actually SLOWER than CPU due to 39 memory copy operations. Only with `cudnn_conv_algo_search: "DEFAULT"` tuning does GPU become faster (4.6x speedup documented in kokoro-onnx#125).

**Decision factors for alternatives:**
1. Does the alternative expose CUDA provider options? (critical for small model perf)
2. Is the alternative mature enough for production use?
3. How much migration effort is required?
4. Does it support the Kokoro ONNX model, or does it bring a different TTS model?

### CRITICAL: Current Integration Surface

**TtsEngine trait** (`server/src/tts.rs:7-11`):
```rust
pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str) -> Result<Vec<i16>>;
    fn set_speed(&self, speed: f32);
}
```

**Touch points (files that use TtsEngine):**
- `server/src/tts.rs` — Trait definition + KokoroTts implementation
- `server/src/main.rs` — Model loading, CLI args (`--tts-model`, `--language`)
- `server/src/server.rs` — Passes `Box<dyn TtsEngine>` to session
- `server/src/session.rs` — `Arc<dyn TtsEngine>`, pipeline synthesis, sentence splitting

**Audio pipeline:**
```
TtsEngine::synthesize(text) → Vec<i16> at 16kHz mono
  → chunked into 4000-sample packets (250ms)
  → sent as TtsAudioChunk(0x83) over TCP
  → TtsEnd(0x84) after last chunk
```

**Resampling (inside KokoroTts only):**
- Kokoro outputs f32 at 24kHz
- Resampled to 16kHz via rubato (SincInterpolation)
- Converted f32→i16 (clamp + scale)
- Alternative engines may output different sample rates — resampling must be adapted

**Model loading:**
- CLI: `--tts-model <path>` points to model directory
- Expected files: `model.onnx`, `voices.bin`, `tokens.txt`, `espeak-ng-data/`, `dict/`, `lexicon-*.txt`
- Different engines will have different model file requirements

**Key constraints for any replacement:**
1. Must be `Send + Sync` (shared via `Arc` across threads)
2. Must provide synchronous API (no async runtime — project uses OS threads + crossbeam)
3. Must output audio convertible to 16kHz mono i16
4. Must support speed control (or the trait method becomes a no-op)
5. Build must work on Linux x86_64 with CUDA 11.x

### Known Alternatives (Pre-Research)

| Alternative | Type | GPU | Notes |
|-------------|------|-----|-------|
| `ort` v2.0.0-rc | ONNX Runtime wrapper | CUDA, TensorRT, DirectML | Full provider control, `cudnn_conv_algo_search` tunable. Requires reimplementing phonemization. |
| `kokorox` | Kokoro Rust impl | NVIDIA GPU via onnxruntime-gpu | Explicit GPU setup. Young project. |
| `Kokoros` | Kokoro Rust impl | Partial | Pure Rust, OpenAI-compatible server. GPU untested. |
| `sherpa-rs` fork | sherpa-onnx wrapper | CUDA (if fixed) | Check for forks exposing provider options. |
| `candle` + model | ML framework | CUDA via candle | Would need TTS model porting. Very different approach. |

### Previous Story Intelligence

From Story 6.2 (Streaming TTS Pipeline):
- `TtsEngine` gained `Sync` bound for `Arc` sharing
- `Box<dyn TtsEngine>` → `Arc<dyn TtsEngine>` conversion in `run_session()`
- Pipeline: producer thread synthesizes per-sentence, consumer sends chunks
- `send_tts_chunks()` / `send_tts_audio()` handle interrupt between chunks
- `split_sentences()` handles text splitting (engine-agnostic)
- 3 mock TTS engines in tests: `MockTtsEngine`, `SentenceMockTtsEngine`, `FailingMockTtsEngine`
- All mocks satisfy `Send + Sync` implicitly (no interior mutability or only `Mutex`)

From Story 6.1 (Barge-in):
- `tts_interrupted: Arc<AtomicBool>` shared flag for interrupt
- Interrupt check happens between chunks, not mid-synthesis
- Any replacement engine must tolerate being called from a thread that may be interrupted

### Project Structure Notes

**Output file:** `_bmad-output/planning-artifacts/tts-gpu-evaluation.md`

**Files NOT to modify (research-only):**
- `server/src/tts.rs` — No changes
- `server/src/session.rs` — No changes
- `server/Cargo.toml` — No dependency changes
- `Makefile` — No build changes

### References

- [Source: server/src/tts.rs] — TtsEngine trait, KokoroTts, resampling pipeline
- [Source: server/src/session.rs] — Pipeline synthesis, Arc<dyn TtsEngine>, send_tts_chunks
- [Source: server/src/main.rs] — Model loading, CLI args
- [Source: server/Cargo.toml] — sherpa-rs v0.6.8, feature flags
- [Source: Makefile] — CUDA=1/all/0, TTS_MODEL path
- [Source: architecture.md] — TtsEngine trait spec, VRAM budget, model loading order
- [Source: 6-2-streaming-tts-pipeline.md] — Pipeline architecture, thread safety requirements
- [Source: kokoro-onnx#125] — cudnn_conv_algo_search 4.6x GPU speedup
- [Source: kokoro-onnx#112] — Kokoro GPU slower than CPU without tuning
- [Source: sherpa-onnx#2138] — CUDA silent crashes
- [Source: ort docs] — ONNX Runtime execution providers
- [Source: kokorox GitHub] — Rust Kokoro with GPU

## Dev Agent Record

### Agent Model Used
claude-opus-4-6

### Debug Log References
N/A (research-only story, no code changes)

### Completion Notes List
- Researched 11 alternatives across 2 categories (same model/different backend, different model)
- Eliminated 8 alternatives (license, size, maturity, abandonment)
- Produced weighted comparison matrix (6 criteria, 6 alternatives)
- Assessed migration effort for top 4 alternatives with per-component breakdown
- Identified `ort + Kokoro` as primary recommendation (score 88/100) — only path to proven GPU 4.6x speedup
- Identified `pocket-tts` as compelling CPU fallback (score 71/100) — RTF 0.17, native Rust, zero native deps
- Identified `Chatterbox Turbo` as future high-quality option (score 74/100) — highest Elo 2055, MIT, but high implementation effort
- Expanded scope beyond original Kokoro-only to include alternative TTS models (user request)
- All 114 tests pass, no code changes made

### Change Log
- `_bmad-output/planning-artifacts/tts-gpu-evaluation.md` — Created evaluation document (deliverable)

### File List
- `_bmad-output/planning-artifacts/tts-gpu-evaluation.md` (CREATED — research deliverable)
