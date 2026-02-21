# Story 6.2: Streaming TTS Pipeline

Status: done

## Story

As a **user**,
I want to hear the AI's response start playing as soon as possible, without waiting for the entire response to be synthesized,
so that conversations feel faster and more natural.

## Acceptance Criteria

1. **Given** the server receives a `ResponseText(0xA1)` from the orchestrator
   **When** the text contains multiple sentences
   **Then** the server splits the text into sentences and synthesizes them sequentially
   **And** the first sentence's audio is sent to the client as `TtsAudioChunk(0x83)` while the second sentence is being synthesized
   **And** audio chunks flow continuously without perceivable gaps between sentences
   **And** `TtsEnd(0x84)` is sent only after the final sentence's audio is complete

2. **Given** a short response (single sentence)
   **When** TTS synthesis completes
   **Then** behavior is identical to current batch implementation (no regression)

3. **Given** a barge-in occurs mid-stream (Story 6.1 interrupt)
   **When** `InterruptTts(0x04)` is received
   **Then** the server stops synthesizing remaining sentences immediately
   **And** no further `TtsAudioChunk` messages are sent for the interrupted response

4. **Given** a paused session
   **When** `ResponseText` arrives during pause
   **Then** behavior is identical to current implementation (skip synthesis, send `TtsEnd`)

5. **Given** TTS synthesis fails on one sentence
   **When** an error occurs
   **Then** the error is logged, already-sent audio is preserved, and `TtsEnd` is sent
   **And** the conversation can continue on the next turn

6. **Given** the complete system
   **When** performing tests
   **Then** unit test: `split_sentences()` correctly splits multi-sentence text
   **And** unit test: sentence-level streaming sends first sentence audio before synthesizing the rest
   **And** unit test: single-sentence text produces same output as batch
   **And** unit test: interrupt stops synthesis of remaining sentences
   **And** `make check` passes with all existing tests + new tests

## Tasks / Subtasks

- [x] Task 1: Add sentence splitting utility (AC: #1, #2)
  - [x] 1.1: Add `split_sentences(text: &str) -> Vec<&str>` in `server/src/session.rs`
  - [x] 1.2: Split on sentence boundaries (`.` `!` `?` followed by whitespace or end-of-string)
  - [x] 1.3: Keep punctuation attached to preceding sentence (important for TTS intonation)
  - [x] 1.4: Handle edge cases: single sentence, no punctuation, empty text, abbreviations
  - [x] 1.5: Add unit tests for `split_sentences()`

- [x] Task 2: Implement pipeline synthesis in `tts_router` (AC: #1, #2, #3, #4)
  - [x] 2.1: Pipeline implemented inline in tts_router (no separate function needed)
  - [x] 2.2: Spawn synthesis producer thread that synthesizes each sentence → sends `Vec<i16>` to a crossbeam channel
  - [x] 2.3: Consumer (tts_router) reads from channel → calls `send_tts_chunks()` for each sentence's samples
  - [x] 2.4: Producer checks `tts_interrupted` between sentences to abort early
  - [x] 2.5: Replace single `tts.synthesize(clean_text)` call with sentence-by-sentence pipeline
  - [x] 2.6: Single-sentence fallback: same path as before (no overhead)

- [x] Task 3: Interrupt handling for streaming pipeline (AC: #3)
  - [x] 3.1: Producer thread stops synthesizing when `tts_interrupted` is set
  - [x] 3.2: Consumer breaks out of channel read when `send_tts_chunks()` returns interrupted
  - [x] 3.3: Ensure `TtsEnd` is sent exactly once (consumer always sends TtsEnd after loop)

- [x] Task 4: Error handling per sentence (AC: #5)
  - [x] 4.1: If synthesis fails on a sentence, producer logs error and breaks
  - [x] 4.2: Already-sent audio from previous sentences is preserved (no rollback)
  - [x] 4.3: `TtsEnd` sent after error (consumer always sends TtsEnd when channel closes)

- [x] Task 5: Tests (AC: #6)
  - [x] 5.1: Unit tests for `split_sentences()` — 9 tests covering all edge cases
  - [x] 5.2: Integration test: `streaming_multi_sentence_sends_all_audio` — pipeline delivers correct total audio
  - [x] 5.3: Integration test: `streaming_single_sentence_same_as_batch` — single sentence matches batch output
  - [x] 5.4: Existing `tts_routing_response_to_audio_chunks` still passes (single-sentence path)
  - [x] 5.5: Integration test: `streaming_interrupt_stops_remaining_sentences` — interrupt stops pipeline
  - [x] 5.6: `make check` passes — 112 tests (24 client + 35 common + 14 orchestrator + 39 server)

## Dev Notes

### CRITICAL: Architecture Overview — How Streaming TTS Works

```
Current (batch):
  Orchestrator sends "Hello. How are you? I'm fine."
  → tts.synthesize(entire_text) → 2-3s BLOCKING synthesis
  → send_tts_audio(all_samples) → stream to client
  Client hears nothing for 2-3s, then playback starts.

With streaming pipeline:
  Orchestrator sends "Hello. How are you? I'm fine."
  → split into: ["Hello.", "How are you?", "I'm fine."]
  → Synthesis thread: synthesize("Hello.") → channel.send(samples)
  → Sender thread:   recv(samples) → send_tts_audio(samples) → CLIENT HEARS AUDIO
  → Synthesis thread: synthesize("How are you?") → channel.send(samples)
  → Sender thread:   recv(samples) → send_tts_audio(samples)
  → ... overlap: sending sentence N while synthesizing sentence N+1
  → After last sentence: send TtsEnd
```

**Key benefit:** Time-to-first-audio drops from `synthesis_time(entire_text)` to `synthesis_time(first_sentence)`. For a typical 3-sentence response, this is ~40-60% reduction.

**Key constraint:** `tts.synthesize()` is blocking ONNX inference. It CANNOT be interrupted mid-sentence. The interrupt happens between sentences (producer stops) and between chunks (existing `send_tts_audio` logic).

### CRITICAL: What Already Works (Do NOT Rewrite)

- `common/src/protocol.rs` — No changes needed. Existing `TtsAudioChunk`/`TtsEnd` protocol handles streaming naturally.
- `client/src/main.rs` — No changes needed. Client already processes `TtsAudioChunk` messages as they arrive. Streaming is transparent to client.
- `client/src/playback.rs` — No changes needed. Playback channel already handles variable-rate chunk arrival.
- `server/src/tts.rs` — `TtsEngine` trait stays unchanged. `synthesize()` is called per-sentence instead of per-response.
- `server/src/session.rs: send_tts_audio()` — Stays unchanged. Called once per sentence instead of once per response.
- `server/src/session.rs: stt_router()` — No changes needed.

### CRITICAL: Files to Modify — ONLY `server/src/session.rs`

**This is a single-file change.** Only `server/src/session.rs` needs modification:

1. Add `split_sentences()` utility function
2. Replace the batch synthesis call in `tts_router()` with a pipeline
3. Add tests

### CRITICAL: Sentence Splitting — `split_sentences()`

```rust
/// Split text into sentences for streaming TTS synthesis.
/// Sentences are split on `.` `!` `?` followed by whitespace or end-of-string.
/// Punctuation stays attached to the preceding sentence (important for TTS intonation).
fn split_sentences(text: &str) -> Vec<&str> {
    // ...
}
```

**Rules:**
- Split on: `.` `!` `?` followed by whitespace or end-of-string
- Keep punctuation with the sentence: `"Hello."` not `"Hello"`
- Trim whitespace from each segment
- Skip empty segments
- Single sentence or no punctuation → return the entire text as one segment
- Do NOT try to handle abbreviations (Mr., e.g., etc.) — not worth the complexity for TTS; worst case is an extra short synthesis call

**Test cases:**
```
"Hello. How are you?" → ["Hello.", "How are you?"]
"I'm fine! Thanks." → ["I'm fine!", "Thanks."]
"Just one sentence" → ["Just one sentence"]
"" → []
"Hello.  Extra  spaces.  " → ["Hello.", "Extra  spaces."]
"Really? Yes! OK." → ["Really?", "Yes!", "OK."]
```

### CRITICAL: Pipeline Architecture in `tts_router()`

**Current code (batch):**
```rust
match tts.synthesize(clean_text) {
    Ok(samples) => {
        let mut w = client_writer.lock()...;
        send_tts_audio(&mut *w, &samples, &tts_interrupted)?;
    }
    Err(e) => { ... }
}
```

**New code (streaming pipeline):**
```rust
let sentences = split_sentences(clean_text);
if sentences.is_empty() {
    let mut w = client_writer.lock()...;
    write_server_msg(&mut *w, &ServerMsg::TtsEnd)?;
    continue;
}

// Single sentence: no pipeline overhead, just synthesize + send
if sentences.len() == 1 {
    match tts.synthesize(sentences[0]) {
        Ok(samples) => {
            let mut w = client_writer.lock()...;
            send_tts_audio(&mut *w, &samples, &tts_interrupted)?;
        }
        Err(e) => { ... }
    }
    continue;
}

// Multiple sentences: pipeline synthesis + send
let (tx, rx) = crossbeam_channel::bounded::<Vec<i16>>(2);
let interrupted_clone = tts_interrupted.clone(); // Need Arc for this

// Producer: synthesize sentences sequentially
let sentence_strs: Vec<String> = sentences.iter().map(|s| s.to_string()).collect();
std::thread::spawn(move || {
    for sentence in &sentence_strs {
        if interrupted_clone.load(Ordering::SeqCst) {
            break;
        }
        match tts.synthesize(sentence) {
            Ok(samples) => {
                if tx.send(samples).is_err() { break; } // consumer dropped
            }
            Err(e) => {
                warn!("[server] TTS synthesis failed for sentence: {e}");
                break;
            }
        }
    }
    drop(tx); // Signal end of production
});

// Consumer: send each sentence's audio as it arrives
let mut w = client_writer.lock()...;
let mut was_interrupted = false;
for samples in rx {
    was_interrupted = send_tts_audio(&mut *w, &samples, &tts_interrupted)?;
    if was_interrupted { break; }
}
if !was_interrupted {
    write_server_msg(&mut *w, &ServerMsg::TtsEnd)?;
}
```

**IMPORTANT — Ownership issue:** The `tts: Box<dyn TtsEngine>` is currently owned by `tts_router`. To send it into a spawned thread, it needs to be `Arc<dyn TtsEngine>` since it's used across multiple `ResponseText` messages. Change the tts_router signature:

```rust
fn tts_router(
    ...
    tts: Arc<dyn TtsEngine>,  // was: Box<dyn TtsEngine>
    ...
)
```

And in `run_session()`:
```rust
let tts: Arc<dyn TtsEngine> = Arc::from(tts);  // Convert Box → Arc
```

This is safe because `TtsEngine: Send` is already required and `KokoroTts` uses internal `Mutex` for thread safety. Add `Sync` to the trait bound:

```rust
pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str) -> Result<Vec<i16>>;
    fn set_speed(&self, speed: f32);
}
```

This works because `KokoroTts` wraps everything in `Mutex`, which is `Sync`.

### CRITICAL: `send_tts_audio` — Minor Fix for Streaming

Currently `send_tts_audio()` always sends `TtsEnd` at the end. For streaming, the consumer needs to send `TtsEnd` only after the last sentence. Two options:

**Option A (recommended):** Split `send_tts_audio` into `send_tts_chunks` (no `TtsEnd`) + explicit `TtsEnd` send:

```rust
/// Send TTS audio chunks. Returns true if interrupted.
/// Does NOT send TtsEnd — caller is responsible.
fn send_tts_chunks(
    writer: &mut impl Write,
    samples: &[i16],
    interrupted: &AtomicBool,
) -> Result<bool> {
    for chunk in samples.chunks(TTS_CHUNK_SIZE) {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(true);
        }
        write_server_msg(writer, &ServerMsg::TtsAudioChunk(chunk.to_vec()))?;
    }
    Ok(false)
}
```

Then the batch wrapper (for single sentence and existing callers) becomes:
```rust
fn send_tts_audio(writer: &mut impl Write, samples: &[i16], interrupted: &AtomicBool) -> Result<bool> {
    let was_interrupted = send_tts_chunks(writer, samples, interrupted)?;
    write_server_msg(writer, &ServerMsg::TtsEnd)?;
    Ok(was_interrupted)
}
```

This keeps the existing API unchanged while enabling streaming control.

**Option B:** Add a `send_tts_end: bool` parameter to `send_tts_audio`. Uglier, avoid.

### CRITICAL: Thread Safety — TtsEngine Must Be Sync

The pipeline spawns a synthesis thread that calls `tts.synthesize()`. The `Arc<dyn TtsEngine>` requires `TtsEngine: Sync`. This is satisfied because:
- `KokoroTts` wraps `sherpa_rs::tts::KokoroTts` in `Mutex` → `Sync`
- `KokoroTts` wraps `speed: f32` in `Mutex` → `Sync`
- `KokoroTts.speaker_id: i32` is immutable → `Sync`

**Change required in `server/src/tts.rs`:**
```rust
pub trait TtsEngine: Send + Sync {  // Add + Sync
```

**Update mock in `server/src/session.rs` tests:**
MockTtsEngine is already `Sync` (no interior mutability, only immutable `sample_count: usize`).

### CRITICAL: What NOT to Do

1. **Do NOT change the `TtsEngine::synthesize()` signature.** Keep it returning `Result<Vec<i16>>`. The streaming happens at the sentence level, not the sample level.
2. **Do NOT add a streaming/iterator API to TtsEngine.** ONNX inference is atomic per call. Streaming within a single sentence is not possible with the current model architecture.
3. **Do NOT modify the client.** Streaming is server-side only. The client already handles `TtsAudioChunk` messages as they arrive.
4. **Do NOT modify the protocol.** Existing `TtsAudioChunk` + `TtsEnd` is sufficient.
5. **Do NOT use async/tokio.** Stay with OS threads + crossbeam-channel.
6. **Do NOT split on every period blindly.** Abbreviations like "Mr." or decimal numbers like "3.5" will occasionally cause extra splits. This is acceptable — worst case is a short extra synthesis call.

### Previous Story Intelligence

From Story 6.1 (Barge-in Interruption):
- `tts_interrupted: Arc<AtomicBool>` shared between `stt_router` (sets flag) and `tts_router` (checks between chunks)
- `send_tts_audio()` returns `bool` indicating interruption — reuse this pattern for per-sentence interrupt check
- `tts_interrupted` is reset to `false` at the start of each `ResponseText` handling — this ensures clean state for each response
- MockTtsEngine uses a simple ramp pattern `(0..sample_count).map(|i| i as i16)` — keep consistent
- Test setup uses `setup_session()` helper — reuse for new integration tests
- Handle `ServerMsg::Text(_)` in test match arms (display texts from "AI:" prefix)

Code patterns established:
- `Arc<AtomicBool>` for shared state between threads
- `Arc<Mutex<BufWriter<TcpStream>>>` for shared TCP writer
- `anyhow::Result` + `.context()` for error handling
- `[server]` prefix in all log messages
- Inline `#[cfg(test)]` modules for unit tests
- `make check` (not raw cargo commands)

### Logging

```
[server] TTS streaming: 3 sentences for 45 chars
[server] TTS sentence 1/3: synthesized 0.3s, 4800 samples (0.30s audio)
[server] TTS sentence 2/3: synthesized 0.5s, 8000 samples (0.50s audio)
[server] TTS sentence 3/3: synthesized 0.4s, 6400 samples (0.40s audio)
[server] TTS streaming complete: 1.2s synthesis for 1.20s audio (3 sentences, 45 chars)
```

### Project Structure Notes

Files to modify:
- `server/src/tts.rs` (MODIFY) — Add `Sync` to `TtsEngine` trait bound
- `server/src/session.rs` (MODIFY) — Add `split_sentences()`, refactor `tts_router()` for pipeline, `send_tts_chunks()`, update `run_session()` for `Arc<dyn TtsEngine>`, add tests

Files NOT to modify:
- `common/src/protocol.rs` — No protocol changes
- `client/src/main.rs` — Client is unaware of streaming
- `client/src/playback.rs` — Playback works as-is
- `server/src/server.rs` — No changes
- `orchestrator/src/*` — Orchestrator is unaware of streaming
- `client/src/vad.rs` — No changes
- `client/src/audio.rs` — No changes

### References

- [Source: architecture.md#Core Architectural Decisions > TTS Engine] — TtsEngine trait with streaming API
- [Source: architecture.md#Gap Resolutions G3] — Audio playback starts immediately on first TtsAudioChunk
- [Source: architecture.md#Audio & Protocol Conventions] — 16kHz mono i16, TtsAudioChunk(0x83), TtsEnd(0x84)
- [Source: epics.md#Story 6.2] — Full acceptance criteria and NFRs
- [Source: epics.md#Epic Dependencies] — Story 6.2 depends on 6.1 for interrupt handling
- [Source: server/src/session.rs] — tts_router(), send_tts_audio(), run_session(), test helpers
- [Source: server/src/tts.rs] — TtsEngine trait, KokoroTts, MockTtsEngine
- [Source: 6-1-barge-in-interruption.md] — Previous story learnings, interrupt flag patterns

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6 (claude-opus-4-6)

### Debug Log References

- `TtsEngine` trait changed from `Send` to `Send + Sync` — required for `Arc<dyn TtsEngine>` which is cloned into the synthesis producer thread. KokoroTts is already Sync (all fields wrapped in Mutex).
- `Box<dyn TtsEngine>` → `Arc<dyn TtsEngine>` conversion in `run_session()` via `Arc::from(tts)`. This allows cloning the engine reference into the producer thread while keeping it available for subsequent ResponseText messages.
- `send_tts_chunks()` extracted from `send_tts_audio()` — sends chunks without TtsEnd, allowing the consumer to control TtsEnd placement (always sent exactly once after channel exhaustion or interrupt).
- Pipeline uses `crossbeam_channel::bounded(2)` — buffer of 2 sentences provides look-ahead without unbounded memory growth.
- Single-sentence responses use the original batch path (no thread spawn overhead).
- Empty text responses immediately send TtsEnd (no synthesis).

### Completion Notes List

- Added `split_sentences()` with 9 unit tests covering: multiple sentences, single sentence, empty, whitespace, extra spaces, mixed punctuation, no-space-after-period, trailing no punctuation
- Added `send_tts_chunks()` — chunk sending without TtsEnd, with 1 unit test
- Added `SentenceMockTtsEngine` — produces `samples_per_char * text.len()` samples, enabling sentence-level verification
- Refactored `tts_router` ResponseText handling into 3 paths: empty → TtsEnd, single sentence → batch, multiple → pipeline
- Pipeline: producer thread synthesizes sentences, sends to bounded(2) channel; consumer sends chunks + TtsEnd
- Producer checks `tts_interrupted` between sentences; consumer checks via `send_tts_chunks` between chunks
- Error in producer: logs and breaks, consumer sends TtsEnd on channel close
- Added `crossbeam-channel = "0.5.15"` dependency to server/Cargo.toml
- 3 streaming integration tests: multi-sentence total audio, single-sentence identity, interrupt stops pipeline
- All 112 tests pass (24 client + 35 common + 14 orchestrator + 39 server), zero regressions
- `make check` clean (fmt + clippy + tests)
- Code review fixes: refactored `send_tts_audio` to call `send_tts_chunks` (DRY), added `FailingMockTtsEngine` + 2 error-handling pipeline tests (AC 5)
- Post-review: 114 tests pass (24 client + 35 common + 14 orchestrator + 41 server)

### File List

- server/src/tts.rs (MODIFIED) — Added `Sync` to `TtsEngine` trait bound
- server/src/session.rs (MODIFIED) — Added `split_sentences()`, `send_tts_chunks()`, `SentenceMockTtsEngine`, `FailingMockTtsEngine`, pipeline synthesis in `tts_router`, `Box` → `Arc` conversion, 15 new tests
- server/Cargo.toml (MODIFIED) — Added `crossbeam-channel = "0.5.15"` dependency

### Change Log

- 2026-02-21: Implemented streaming TTS pipeline (Story 6.2) — sentence-level splitting, producer/consumer pipeline with crossbeam channel, interrupt support between sentences, TtsEngine Send+Sync, 13 new tests. All 112 tests passing.
- 2026-02-21: Code review fixes — refactored `send_tts_audio` to call `send_tts_chunks` (DRY elimination), added 2 pipeline error-handling tests (AC 5 coverage). 114 tests passing.
