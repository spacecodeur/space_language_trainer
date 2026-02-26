---
title: 'Fix TTS audio pops at chunk boundaries'
slug: 'fix-tts-audio-pops'
created: '2026-02-26'
status: 'done'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rubato-1.0.1, audioadapter_buffers, cpal, crossbeam-channel, sherpa-rs]
files_to_modify: [client/src/audio.rs, client/src/main.rs, server/src/session.rs]
code_patterns: [resampler-closure-with-mutable-state, chunk-streaming-producer-consumer, empty-slice-flush-convention]
test_patterns: [unit-tests-in-mod, sine-wave-test-signals]
---

# Tech-Spec: Fix TTS audio pops at chunk boundaries

**Created:** 2026-02-26

## Overview

### Problem Statement

Users hear micro audio pops/clicks every ~250ms (2-3 words) during TTS playback. The root cause is the client-side resampler (16kHz → 48kHz) zero-padding partial sub-chunks at every TCP chunk boundary. Since TTS_CHUNK_SIZE (4000 samples) is not a multiple of the resampler's internal chunk_size (1024), 928 samples are zero-padded to 1024 on every single TCP chunk, corrupting the sinc filter state and producing audible discontinuities.

A secondary issue exists at sentence boundaries where independently synthesized sentences are directly concatenated without crossfading.

### Solution

1. **Primary fix**: Replace zero-padding with a carry-over buffer in the client resampler. Leftover samples (< 1024) from one TCP chunk are prepended to the next chunk, eliminating zero-padding during normal streaming. An empty-slice call (`&[]`) signals flush for stream end.
2. **Secondary fix**: Add a short crossfade at sentence boundaries on the server side to eliminate amplitude discontinuities between independently synthesized sentences.

### Scope

**In Scope:**
- Client resampler carry-over buffer (client/src/audio.rs)
- Client resampler flush on TtsEnd (client/src/main.rs)
- Sentence boundary crossfade (server/src/session.rs)
- Unit tests for both fixes

**Out of Scope:**
- Changing TTS codec or engine
- Modifying the wire protocol
- Changing server-side sample rate (stays 16kHz)
- Changing TTS_CHUNK_SIZE constant
- Server-side resampler (24→16kHz) — processes full sentences, zero-padding only at end (inaudible)

## Context for Development

### Codebase Patterns

- **ResamplerFn type**: `Box<dyn FnMut(&[i16]) -> Vec<i16>>` closure that captures mutable rubato `Async<f64>` resampler. Created once, called per TCP chunk. State persists across calls.
- **Resampler internal chunking**: rubato Async resampler uses fixed 1024-frame chunks internally (`FixedAsync::Input`). Current code processes input in 1024-frame sub-chunks with zero-padding on the last partial sub-chunk.
- **Empty-slice convention**: A call with `&[]` will be the new "flush" signal. This is never sent during normal streaming (TTS chunks always have > 0 samples).
- **Playback pipeline**: `tcp_reader_loop` → optional resample → `playback_tx` channel (bounded 32) → cpal callback with leftover buffer
- **Server streaming**: `split_sentences()` → producer thread synthesizes sentences → `crossbeam_channel::bounded(2)` → consumer thread calls `send_tts_chunks()` per sentence → `TtsEnd`
- **Crossfade math**: Linear crossfade over N samples: `out[i] = prev[i] * (1 - i/N) + next[i] * (i/N)` applied to i16 samples at junction point

### Files to Reference

| File | Purpose | Key Lines |
| ---- | ------- | --------- |
| client/src/audio.rs:88-175 | `create_resampler()` closure — primary fix target | Lines 131-171: sub-chunk loop with zero-padding |
| client/src/main.rs:736-768 | `tcp_reader_loop` TtsAudioChunk/TtsEnd handlers — flush call site | Lines 740-751: resample + send, Lines 753-762: TtsEnd |
| server/src/session.rs:352-365 | Consumer loop for streaming multi-sentence TTS — crossfade site | Lines 358-363: `for samples in rx` loop |
| server/src/session.rs:483-495 | `send_tts_chunks()` — sends 4000-sample chunks | |
| server/src/tts.rs:147-215 | Server 24→16kHz resampler (reference, not modified) | Same pattern as client |
| client/src/playback.rs:50-108 | cpal playback callback (context only) | |

### Technical Decisions

- **Carry-over vs larger chunk_size**: Carry-over is cleaner — changing chunk_size to 4000 would couple the resampler to TTS_CHUNK_SIZE and still break on last-chunk-of-sentence. Carry-over is universal.
- **Empty-slice flush vs trait refactor**: Empty-slice convention keeps `ResamplerFn` type unchanged (just a `FnMut`). A trait would be more explicit but requires changing all usage sites — overkill for this fix.
- **Crossfade length**: 160 samples = 10ms at 16kHz. Standard for speech. Long enough to eliminate clicks, short enough to be imperceptible. Shorter than any phoneme.
- **Crossfade location**: Server-side in the consumer loop (session.rs), not in `send_tts_chunks()`, because the consumer receives whole sentences and knows the boundary.
- **Linear crossfade**: Sufficient for speech. More complex curves (equal-power) are overkill here.

## Implementation Plan

### Tasks

- [x] Task 1: Add carry-over buffer to client resampler
  - File: `client/src/audio.rs`
  - Action: Modify the closure returned by `create_resampler()` to maintain a `leftover: Vec<f64>` buffer between calls. Replace the current sub-chunk loop (lines 131-171) with:
    1. Convert input i16 → f64 mono (unchanged)
    2. Prepend `leftover` to the converted input
    3. Process only full `chunk_size` (1024) frames through the rubato resampler
    4. Store remaining frames (< 1024) in `leftover` for the next call
    5. If input is empty (`samples.is_empty()`), flush: process leftover with zero-padding to `chunk_size`, then clear leftover. This is the only path where zero-padding occurs.
    6. Convert output f64 → i16 (unchanged)
  - Notes: The `leftover` Vec is captured by the `move` closure alongside the rubato resampler. Max leftover size is 1023 samples (64ms at 16kHz). The no-op path (same rate, mono) remains unchanged — returns `samples.to_vec()` without carry-over.

- [x] Task 2: Flush resampler on TtsEnd in tcp_reader_loop
  - File: `client/src/main.rs`
  - Action: In the `ServerMsg::TtsEnd` match arm of `tcp_reader_loop`, before setting `is_playing = false`, call the resampler with an empty slice to flush any leftover samples:
    ```
    if let Some(r) = &mut resample {
        let tail = r(&[]);
        if !tail.is_empty() {
            // Accumulate for replay
            if let Ok(mut buf) = last_tts_audio.lock() ... { buf.extend_from_slice(&tail); }
            let _ = playback_tx.send(tail);
        }
    }
    ```
  - Notes: Must happen BEFORE `is_playing.store(false)` and BEFORE the `[3] Replay` hint is shown. The replay buffer accumulation must also include the tail samples.

- [x] Task 3: Add sentence boundary crossfade in server TTS consumer
  - File: `server/src/session.rs`
  - Action: In the consumer section of the streaming multi-sentence TTS (lines 352-365), maintain a `prev_tail: Option<Vec<i16>>` holding the last `CROSSFADE_LEN` samples of the previous sentence. For each new sentence (except the first):
    1. If `prev_tail` is `Some` and new sentence has >= `CROSSFADE_LEN` samples:
       - Apply linear crossfade: for i in 0..CROSSFADE_LEN: `samples[i] = prev_tail[i] * (1.0 - t) + samples[i] * t` where `t = i as f32 / CROSSFADE_LEN as f32`
    2. Send the (possibly crossfaded) samples via `send_tts_chunks()`
    3. Save the last `CROSSFADE_LEN` samples of this sentence as `prev_tail`
  - Notes: Add `const CROSSFADE_LEN: usize = 160;` (10ms at 16kHz). Crossfade operates on i16 samples — use i32 intermediate to avoid overflow. The first sentence has no crossfade (no predecessor). If a sentence has fewer samples than CROSSFADE_LEN, skip crossfade for that boundary.

- [x] Task 4: Add unit tests for carry-over resampler
  - File: `client/src/audio.rs`
  - Action: Add tests in the existing `#[cfg(test)] mod tests` block:
    1. `resampler_carry_over_no_discontinuity`: Create a 440Hz sine wave at 16kHz (2 seconds = 32000 samples). Process it in chunks of 4000 through a 16kHz→48kHz resampler. Concatenate all outputs + flush. Verify: no sample-to-sample jump exceeds a threshold (e.g., 3000 i16 units) at chunk boundaries, meaning no pop. Compare against processing the full signal in one call — output lengths should be within a small margin.
    2. `resampler_flush_produces_remaining_samples`: Create a short signal (e.g., 500 samples, less than chunk_size). Call resampler once, then flush. Verify total output is non-empty and approximately `500 * ratio` samples.
    3. `resampler_carry_over_matches_single_pass`: Process 8000 samples in 4000+4000 chunks + flush vs single 8000-sample call. Verify outputs have similar length (within 5% margin).
  - Notes: Existing tests `resampler_noop_mono` and `resampler_48k_to_16k` must still pass.

- [x] Task 5: Add unit test for sentence crossfade
  - File: `server/src/session.rs`
  - Action: Add a test in the existing `#[cfg(test)] mod tests` block:
    1. `crossfade_smooths_sentence_boundary`: Create two Vec<i16> representing sentences: sentence A ending at amplitude 10000, sentence B starting at amplitude -5000. Apply the crossfade function. Verify: the first CROSSFADE_LEN samples of sentence B transition smoothly from A's tail level toward B's original values. No sample jump > some threshold at the boundary.
  - Notes: Extract the crossfade logic into a helper function `apply_crossfade(prev_tail: &[i16], samples: &mut [i16])` to make it independently testable.

### Acceptance Criteria

- [ ] AC1: Given a client with output_rate != 16kHz (e.g., 48kHz), when TTS audio is streamed in 4000-sample chunks, then no audible pop/click is heard at chunk boundaries (carry-over eliminates zero-padding during streaming).
- [ ] AC2: Given TTS streaming in progress, when TtsEnd is received, then the resampler is flushed and any remaining samples are played back (no audio truncation at end of response).
- [ ] AC3: Given a multi-sentence TTS response, when sentences are streamed sequentially, then no audible click is heard at sentence boundaries (crossfade applied).
- [ ] AC4: Given a single-sentence TTS response, when audio is streamed, then no crossfade is applied and playback is identical to current behavior.
- [ ] AC5: Given a sine wave processed through the resampler in 4000-sample chunks + flush, when compared to single-pass processing, then total output length matches within 5% margin and no discontinuity exceeds threshold at chunk boundaries.
- [ ] AC6: Given the no-op resampler path (output_rate == 16kHz), when audio is processed, then behavior is unchanged (pass-through, no carry-over overhead).
- [ ] AC7: Given all existing tests, when `make check` is run, then all tests pass with no warnings.

## Additional Context

### Dependencies

- `rubato` 1.0.1 (already in both client/Cargo.toml and server/Cargo.toml)
- `audioadapter_buffers` (already a transitive dependency, used in audio.rs)
- No new dependencies needed

### Testing Strategy

- **Unit tests** (automated):
  - Carry-over continuity: sine wave chunked vs single-pass (client/src/audio.rs)
  - Flush completeness: short signal flush produces remaining samples (client/src/audio.rs)
  - Crossfade smoothness: amplitude transition at sentence boundary (server/src/session.rs)
- **Regression tests** (automated):
  - Existing `resampler_noop_mono` and `resampler_48k_to_16k` must pass unchanged
  - All 148+ existing workspace tests must pass
- **Manual testing**:
  - Play a multi-sentence TTS response on Bluetooth speaker (48kHz) — listen for pops
  - Play a single short response — verify no audio truncation
  - Verify replay feature still works correctly with flushed audio included

### Notes

- **Risk — Carry-over latency**: Max 1023 samples carried over = 64ms at 16kHz. This delay is imperceptible for speech. The first chunk of a new response processes immediately (carry-over is empty after flush).
- **Risk — Crossfade on very short sentences**: If a sentence has < 160 samples (~10ms), crossfade is skipped. This is safe because such short sentences are rare and the TTS engine typically produces at least 100ms of audio per sentence.
- **Future consideration**: The server-side resampler (24→16kHz in tts.rs) has the same zero-padding pattern on the last sub-chunk of each sentence. Not audible now, but if it becomes an issue, the same carry-over fix can be applied.
- **User's device**: Bose Flex 2 SoundLink (Bluetooth, 48kHz s16le 2ch). Client resampler 16kHz→48kHz is active.
