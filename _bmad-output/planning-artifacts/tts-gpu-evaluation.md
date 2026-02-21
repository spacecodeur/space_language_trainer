# TTS GPU Alternatives Evaluation

**Story:** 6-3 | **Date:** 2026-02-21 | **Status:** Complete

## Executive Summary

The current TTS setup (sherpa-rs v0.6.8 + Kokoro 82M) works well on CPU (~0.3-0.5s per sentence) but CUDA is broken (silent crashes, `sherpa-onnx#2138`). sherpa-rs does not expose CUDA provider options (`cudnn_conv_algo_search`) which are the only proven way to make Kokoro faster on GPU than CPU (4.6x speedup per `kokoro-onnx#125`).

This evaluation covers two categories of alternatives:
- **Category A** — Same Kokoro model, different ONNX backend (ort, kokorox, Kokoros)
- **Category B** — Different TTS model entirely (Pocket TTS, Chatterbox Turbo, F5-TTS)

**Recommendation:** Implement `ort` crate with Kokoro ONNX model (Category A) as the primary path, with Pocket TTS as a compelling CPU-first fallback.

---

## 1. Alternatives Catalog

### Category A: Kokoro Model, Alternative Backend

| Name | Version | Engine | GPU Backends | crates.io | Stars | Last Release | Activity |
|------|---------|--------|-------------|-----------|-------|-------------|----------|
| **ort** | 2.0.0-rc.11 | ONNX Runtime wrapper | CUDA, TensorRT, DirectML, CoreML, ROCM | Yes (6.1M dl) | 2K+ | 2025-04 | Very active, 3 open issues |
| **kokorox** | 0.2.2 | ort + Kokoro pipeline | CUDA (via external libonnxruntime) | No (git only) | ~50 | 2025-02 | Low activity |
| **Kokoros** | 0.1.0 | ort + Kokoro pipeline | Partial (untested) | No (git only) | 713 | 2025-02 | Moderate, OpenAI-compat server focus |
| **kokoroxide** | 0.1.5 | ort + Kokoro pipeline | Via ort providers | Yes (very early) | <10 | 2025-01 | Minimal |
| **sherpa-rs** | 0.6.8 | sherpa-onnx C FFI | CUDA (crashes) | Yes | 30 | 2025-01 | Active but CUDA broken |

### Category B: Alternative TTS Model

| Name | Params | Engine / Format | GPU | License | Quality (Elo) | RTF (CPU) | Rust Crate |
|------|--------|----------------|-----|---------|---------------|-----------|------------|
| **Pocket TTS (Kyutai)** | 100M | Candle (native Rust) | CPU only (Candle CUDA possible) | MIT | 2016 | 0.17 | `pocket-tts` v0.6.2 |
| **Chatterbox Turbo (Resemble)** | 350M | ONNX (official export) | CUDA via ort | MIT | 2055 (highest) | ~0.5 (GPU) | None (use ort) |
| **F5-TTS** | 335M | ONNX (community export) | CUDA via ort | CC-BY-NC | ~1950 | ~0.7 (GPU) | None (use ort) |
| **Orpheus** | 3B (nano 150M TBA) | PyTorch | CUDA | Apache 2.0 | ~1900 | N/A | None |
| **Piper** | <10M | ONNX | CPU focused | MIT | ~1700 | <0.1 | None |

### Eliminated Alternatives

| Name | Reason for elimination |
|------|----------------------|
| **Coqui/XTTS** | Company defunct. CPML license (non-commercial). No maintenance. |
| **Parler-TTS** | 2.3B params — too large for real-time. No ONNX export. |
| **MARS5** | 750M params, poor quality reports, abandoned. |
| **Dia (Nari Labs)** | 1.6B params — too large. Dialogue-focused, not general TTS. |
| **OuteTTS** | Very slow inference (RTF >1.0 on CPU). Niche. |
| **Spark-TTS** | CC-BY-NC-SA license. 500M params. |
| **voirs** | Alpha-quality, skeptical maturity. |
| **Ollama** | Does not natively support TTS. Audio models (Ultravox, etc.) are STT, not TTS. |

---

## 2. Comparison Matrix

### Criteria Weights

| Criterion | Weight | Rationale |
|-----------|--------|-----------|
| GPU performance potential | 25% | Primary goal of this investigation |
| Maturity & stability | 20% | Production reliability matters |
| API compatibility (TtsEngine trait) | 20% | Lower migration effort = faster delivery |
| Audio quality | 15% | Must be at least as good as current Kokoro |
| License | 10% | Must be permissive (MIT/Apache) |
| Build complexity | 10% | CUDA toolkit, native deps, CI impact |

### Evaluation Matrix

| Criterion (weight) | ort + Kokoro | kokorox | Kokoros | pocket-tts | Chatterbox (ort) | F5-TTS (ort) |
|---------------------|-------------|---------|--------|------------|-----------------|-------------|
| **GPU perf (25%)** | **A** — Full `cudnn_conv_algo_search` control. Proven 4.6x speedup. | C — Uses ort internally but doesn't expose CUDA options. | C — Same limitation as kokorox. | D — CPU-only (Candle CUDA theoretically possible but unproven for this model). | B — 350M benefits more from GPU than 82M Kokoro. No tuning data yet. | B — Similar to Chatterbox but less tested. |
| **Maturity (20%)** | **A** — 6.1M downloads, 2K stars, Microsoft-backed ONNX Runtime underneath. | D — Not on crates.io, <100 stars, v0.2.x. | C — Not on crates.io, 713 stars, v0.1.x. | B — On crates.io (v0.6.2), ~200 downloads, uses battle-tested Candle. | C — Model is new (2025-06), official ONNX exists, but no Rust integration yet. | D — Community ONNX, less tested. |
| **API compat (20%)** | B — Must reimplement phonemization pipeline (espeak-ng + Misaki → token IDs). `Session` is `Send + Sync`. Sync `run()` API. | **A** — Drop-in Kokoro replacement. Same model, handles phonemization. | **A** — Same as kokorox. | **A** — Native Rust crate with `generate()` → `Vec<f32>`. Easy to wrap in TtsEngine. | C — Need to build full inference pipeline from ONNX. No existing Rust wrapper. | C — Same as Chatterbox. |
| **Quality (15%)** | B — Same Kokoro model (Elo ~2000). | B — Same Kokoro model. | B — Same Kokoro model. | B — Elo 2016, comparable to Kokoro. | **A** — Elo 2055, highest quality. Voice cloning. | B — Elo ~1950, good quality. |
| **License (10%)** | **A** — MIT (ort) + Apache (Kokoro weights). | A — MIT. | A — MIT. | **A** — MIT. | **A** — MIT. | **F** — CC-BY-NC (non-commercial). Disqualified. |
| **Build complexity (10%)** | B — Pre-built CUDA 12 binaries via `ort`. Needs CUDA toolkit for GPU. Feature flag switch. | C — Requires external libonnxruntime install. Not on crates.io (git dep). | C — Same as kokorox. | **A** — Pure Rust (Candle). `cargo add pocket-tts`. No native deps. | B — Same as ort (uses ort underneath). | B — Same as ort. |

### Weighted Scores

| Alternative | GPU (25) | Maturity (20) | API (20) | Quality (15) | License (10) | Build (10) | **Total** |
|------------|---------|-------------|---------|-------------|-------------|-----------|-----------|
| **ort + Kokoro** | 25 | 20 | 14 | 12 | 10 | 7 | **88** |
| **pocket-tts** | 5 | 14 | 20 | 12 | 10 | 10 | **71** |
| **Chatterbox (ort)** | 18 | 12 | 12 | 15 | 10 | 7 | **74** |
| **kokorox** | 8 | 5 | 20 | 12 | 10 | 5 | **60** |
| **Kokoros** | 8 | 8 | 20 | 12 | 10 | 5 | **63** |
| **F5-TTS (ort)** | 18 | 5 | 12 | 12 | 0 | 7 | **54** — DISQUALIFIED (license) |

---

## 3. Migration Effort Assessment

### 3.1 ort + Kokoro (Recommended)

**Overall: MEDIUM**

| Component | Effort | Details |
|-----------|--------|---------|
| `server/src/tts.rs` | HIGH | New `OrtKokoroTts` struct. Must reimplement: (1) phonemization via espeak-ng FFI or Misaki crate, (2) token encoding, (3) ONNX inference via `ort::Session::run()`, (4) audio post-processing. Current `KokoroTts` is ~100 lines; replacement will be ~200-300 lines. |
| `server/Cargo.toml` | LOW | Replace `sherpa-rs` with `ort = { version = "2.0.0-rc.11", features = ["cuda"] }`. Add `espeak-ng-sys` or `misaki` for phonemization. |
| `server/src/main.rs` | LOW | Model loading path changes slightly. Same CLI args. Feature flag `cuda-tts` now controls ort CUDA provider instead of sherpa. |
| `server/src/session.rs` | NONE | No changes. Uses `Arc<dyn TtsEngine>` — backend-agnostic. |
| `server/src/server.rs` | NONE | No changes. Passes `Box<dyn TtsEngine>`. |
| `Makefile` | LOW | Update CUDA feature flags. ort downloads pre-built binaries, simplifying build. |
| Tests | LOW | Mock engines unchanged. Integration test with real model needs model path update. |
| Resampling | LOW | Kokoro ONNX outputs 24kHz — same resampling pipeline applies. |

**Key risk:** Phonemization reimplementation. Kokoro requires text → phonemes → token IDs. sherpa-rs handles this internally. With ort, we must do it ourselves. Options:
1. **espeak-ng FFI** — Battle-tested, same phonemizer Kokoro was trained with. Requires `espeak-ng` system library.
2. **Misaki crate** — Pure Rust phonemizer designed for Kokoro. Less mature but no native deps.
3. **Port from kokorox/Kokoros** — These projects already solved this. Can reference their implementation.

**GPU tuning path:**
```rust
let cuda = ort::CUDAExecutionProvider::default()
    .with_conv_algorithm_search(ort::CudnnConvAlgoSearch::Default);
let session = ort::Session::builder()?
    .with_execution_providers([cuda.into()])?
    .commit_from_file("model.onnx")?;
```

### 3.2 pocket-tts (CPU Fallback)

**Overall: LOW**

| Component | Effort | Details |
|-----------|--------|---------|
| `server/src/tts.rs` | LOW | New `PocketTtsEngine` struct wrapping `pocket_tts::Tts`. ~50 lines. Native f32 output, needs sample rate check (likely 24kHz → resample to 16kHz). |
| `server/Cargo.toml` | LOW | `cargo add pocket-tts`. Pure Rust, no native deps. |
| `server/src/main.rs` | LOW | Different model loading (Pocket TTS model files vs Kokoro directory). |
| `server/src/session.rs` | NONE | No changes. |
| Resampling | LOW | Check Pocket TTS output sample rate. May need different ratio. |
| Tests | NONE | Mock engines unchanged. |

**Key advantage:** No GPU complexity. CPU RTF of 0.17 means it's faster than real-time on CPU alone. Frees GPU entirely for Whisper STT.

**Key risk:** Less community adoption. Audio quality is comparable to Kokoro but unverified in our pipeline. No speed control API (set_speed becomes no-op).

### 3.3 Chatterbox Turbo (Future Option)

**Overall: HIGH**

| Component | Effort | Details |
|-----------|--------|---------|
| `server/src/tts.rs` | HIGH | Must build full ONNX inference pipeline from scratch. Chatterbox has multiple ONNX files (encoder, decoder, vocoder). No existing Rust wrapper. |
| `server/Cargo.toml` | MEDIUM | ort + custom pipeline deps. |
| Model files | MEDIUM | Download from HuggingFace. Different structure than Kokoro. |
| Resampling | UNKNOWN | Output sample rate TBD. |

**Key advantage:** Highest quality (Elo 2055). Voice cloning capability. MIT license.
**Key risk:** No existing Rust implementation. Building from ONNX files is significant effort. 350M params — needs GPU for real-time.

### 3.4 kokorox / Kokoros

**Overall: LOW (migration) but LOW (value)**

| Component | Effort | Details |
|-----------|--------|---------|
| Migration | LOW | Drop-in Kokoro replacement. Same model, similar API. |
| Value | LOW | Neither exposes CUDA provider options. Same GPU limitation as sherpa-rs. No improvement over current setup. |

**Not recommended** — migrating to a less mature wrapper that doesn't solve the core problem.

---

## 4. Recommendation

### Ranked Alternatives

| Rank | Alternative | Score | Rationale |
|------|------------|-------|-----------|
| **1** | **ort + Kokoro** | 88 | Only path to proven GPU speedup (4.6x via `cudnn_conv_algo_search`). Most mature ONNX runtime. Full provider control. Medium migration effort but highest payoff. |
| **2** | **Chatterbox Turbo (ort)** | 74 | Highest audio quality. MIT. But high implementation effort (no Rust wrapper) and 350M params require GPU. Better as a future upgrade after ort infrastructure is in place. |
| **3** | **pocket-tts** | 71 | Excellent CPU-first option. Trivial migration. But doesn't solve GPU goal. Best as a complementary backend — frees GPU for Whisper while providing fast CPU TTS. |
| **4** | **Kokoros** | 63 | Easy migration but doesn't improve GPU situation. |
| **5** | **kokorox** | 60 | Same as Kokoros but less mature. |
| — | **F5-TTS** | 54 | Disqualified: CC-BY-NC license. |

### Recommended Implementation (Story 6-4)

**Primary: Migrate to `ort` + Kokoro ONNX model**

This is the only alternative that:
1. Exposes `cudnn_conv_algo_search` for proven GPU speedup
2. Is production-mature (6.1M downloads, Microsoft-backed)
3. Uses the same Kokoro model (no quality regression)
4. Provides `Send + Sync` session (compatible with our threading model)
5. Downloads pre-built CUDA binaries (simpler build than sherpa-onnx)

**Implementation approach:**
1. Add `ort` dependency with CUDA feature
2. Port phonemization from kokorox/Kokoros reference code (they solved this already)
3. Build `OrtKokoroTts` implementing `TtsEngine` trait
4. Keep `KokoroTts` (sherpa-rs) behind feature flag as fallback
5. Add `cudnn_conv_algo_search: DEFAULT` configuration
6. Benchmark CPU vs GPU with tuning

**Optional companion: Add `pocket-tts` as CPU backend**

If ort migration proves complex, pocket-tts can serve as an immediate CPU upgrade:
- RTF 0.17 vs current ~0.3-0.5 — faster on CPU
- No native deps — simpler build
- Frees GPU budget entirely for Whisper
- Could coexist: `--tts-backend ort|pocket|sherpa`

### Risks and Blockers

| Risk | Severity | Mitigation |
|------|----------|------------|
| Phonemization reimplementation | MEDIUM | Reference kokorox/Kokoros code. Both already solved espeak-ng → token pipeline. |
| ort v2.0 is still RC | LOW | 6.1M downloads on RC. Stable enough for production use. |
| CUDA 12 vs CUDA 11 compatibility | LOW | ort supports CUDA 11.x and 12.x via different feature flags. |
| Kokoro GPU may still be slow without tuning | LOW | `cudnn_conv_algo_search` is the proven fix. ort exposes it directly. |
| pocket-tts audio quality untested in pipeline | LOW | Easy to prototype — add crate, wrap in TtsEngine, A/B test. |

### Open Questions

1. **espeak-ng vs Misaki for phonemization?** — espeak-ng is battle-tested but requires system library. Misaki is pure Rust but less mature. Evaluate during implementation.
2. **Should we support multiple TTS backends simultaneously?** — Feature flags (`--tts-backend`) would allow users to choose. Minimal extra code if TtsEngine trait is already abstracted.
3. **Pocket TTS output sample rate?** — Need to verify during implementation. Resampling pipeline may need adjustment.

---

## Appendix: Research Sources

- ort crate: [crates.io/crates/ort](https://crates.io/crates/ort), [GitHub pykeio/ort](https://github.com/pykeio/ort)
- kokorox: [GitHub thewh1teagle/kokorox](https://github.com/thewh1teagle/kokorox)
- Kokoros: [GitHub lucaelin/Kokoros](https://github.com/lucaelin/Kokoros)
- pocket-tts: [crates.io/crates/pocket-tts](https://crates.io/crates/pocket-tts)
- Chatterbox: [GitHub resemble-ai/chatterbox](https://github.com/resemble-ai/chatterbox)
- Kokoro ONNX GPU tuning: [kokoro-onnx#125](https://github.com/thewh1teagle/kokoro-onnx/issues/125)
- Kokoro GPU slower than CPU: [kokoro-onnx#112](https://github.com/thewh1teagle/kokoro-onnx/issues/112)
- sherpa-onnx CUDA crashes: [sherpa-onnx#2138](https://github.com/k2-fsa/sherpa-onnx/issues/2138)
- TTS Arena Leaderboard: [HuggingFace TTS Arena](https://huggingface.co/spaces/Pendrokar/TTS-Spaces-Arena)
