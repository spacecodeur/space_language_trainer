# Story 6.1: Barge-in Interruption

Status: done

## Story

As a **user**,
I want to interrupt the AI's spoken response by starting to speak,
so that I can naturally take my turn without waiting for the AI to finish talking.

## Acceptance Criteria

1. **Given** the AI is currently speaking (TTS audio playing on client) and the user is in listening mode (hotkey held)
   **When** the user starts speaking (VAD detects sustained voice activity)
   **Then** the client immediately stops TTS audio playback (clears playback buffer)
   **And** the client sends an `InterruptTts(0x04)` message to the server
   **And** the server aborts sending any remaining TTS audio chunks for the current response
   **And** the server sends `TtsEnd(0x84)` to signal the end of the interrupted response
   **And** barge-in detection responds within 300ms of user voice onset

2. **Given** barge-in has occurred
   **When** the user finishes speaking
   **Then** the user's speech is captured, transcribed via STT, and processed as a new conversation turn
   **And** the conversation continues naturally from the user's interruption

3. **Given** the AI is speaking and the user does NOT speak
   **When** background noise occurs (cough, door slam, speaker echo)
   **Then** VAD does not trigger barge-in (only sustained speech triggers interruption)
   **And** TTS playback continues uninterrupted

4. **Given** the system with barge-in support
   **When** the AI finishes speaking normally (no interruption)
   **Then** behavior is identical to current implementation (no regression)

5. **Given** the new `InterruptTts(0x04)` protocol message
   **When** encoding and decoding it
   **Then** round-trip serialization test passes (empty payload, client→server tag namespace)

6. **Given** the complete system
   **When** performing integration and manual E2E tests
   **Then** mock TTS playback + simulated VAD trigger → playback stops and InterruptTts is sent
   **And** manual E2E: mid-AI-response, start speaking, verify AI stops and processes the new input

## Tasks / Subtasks

- [x] Task 1: Add `InterruptTts(0x04)` protocol message (AC: #5)
  - [x] 1.1: Add `InterruptTts` variant to `ClientMsg` enum in `common/src/protocol.rs`
  - [x] 1.2: Add write support (tag `0x04`, empty payload) in `write_client_msg`
  - [x] 1.3: Add read support (tag `0x04`) in `read_client_msg`
  - [x] 1.4: Add round-trip serialization test `round_trip_interrupt_tts`
  - [x] 1.5: Add multi-message stream test including InterruptTts

- [x] Task 2: Server-side interrupt handling (AC: #1, #4)
  - [x] 2.1: Add `tts_interrupted: Arc<AtomicBool>` shared between `stt_router` and `tts_router` in `run_session()`
  - [x] 2.2: Handle `ClientMsg::InterruptTts` in `stt_router`: set `tts_interrupted` flag, log at info level
  - [x] 2.3: Modify `send_tts_audio()` to accept `&AtomicBool` interrupt flag, check between each chunk send → abort + send TtsEnd if set
  - [x] 2.4: In `tts_router`, reset `tts_interrupted` at the start of each `ResponseText` handling
  - [x] 2.5: Integration test: send ResponseText → send InterruptTts mid-stream → verify truncated audio + TtsEnd

- [x] Task 3: Client playback state tracking (AC: #1, #4)
  - [x] 3.1: Add `is_playing: Arc<AtomicBool>` in client `main.rs`, pass to `tcp_reader_loop`
  - [x] 3.2: In `tcp_reader_loop`: set `is_playing = true` on first `TtsAudioChunk`, set `is_playing = false` on `TtsEnd`
  - [x] 3.3: Add playback clear mechanism: `playback_clear` AtomicBool flag checked in playback callback — drains channel + leftover + outputs silence

- [x] Task 4: Client barge-in detection and interrupt signaling (AC: #1, #2, #3)
  - [x] 4.1: In main audio loop: when `is_listening && is_playing`, run VAD on captured audio
  - [x] 4.2: Add barge-in voice detection: require `BARGE_IN_THRESHOLD` (3 consecutive audio chunks with voice) before triggering
  - [x] 4.3: On barge-in trigger: send `ClientMsg::InterruptTts` to server, set `is_playing = false`, flush playback buffer
  - [x] 4.4: After interrupt, continue VAD normally — speech is captured and sent as `AudioSegment` when complete
  - [x] 4.5: Add `barge_in_frames: u32` counter to track consecutive voice chunks during playback

- [x] Task 5: Tests and validation (AC: #5, #6)
  - [x] 5.1: Protocol round-trip tests for `InterruptTts` (unit, inline `#[cfg(test)]`)
  - [x] 5.2: Server integration test: barge-in mid-TTS stream (`interrupt_tts_aborts_audio_stream`)
  - [x] 5.3: Server integration test: InterruptTts during synthesis (covered by 5.2 — flag set before chunks sent)
  - [x] 5.4: Server integration test: normal flow (no interrupt) still works unchanged (`interrupt_tts_normal_flow_without_interrupt`)
  - [x] 5.5: Run `make check` — all 93 tests pass, no regressions

## Dev Notes

### CRITICAL: Architecture Overview — How Barge-in Works

```
Current flow (no barge-in):
  User speaks → VAD → AudioSegment → Server STT → Orchestrator → Claude
  Claude responds → ResponseText → Server TTS synthesize() → send chunks → Client plays

With barge-in:
  ...Client is playing TTS audio...
  User starts speaking → VAD detects voice during playback
  Client: clear playback + send InterruptTts(0x04)
  Server: stt_router receives InterruptTts → sets tts_interrupted flag
  Server: tts_router sees flag between chunks → aborts remaining chunks + sends TtsEnd
  User finishes speaking → VAD emits segment → AudioSegment → normal flow
```

**Key constraint:** `tts.synthesize()` is blocking ONNX inference (~1-3s). It CANNOT be interrupted mid-computation. The interrupt happens during the chunk streaming phase (`send_tts_audio`). For typical responses, synthesis produces ~32k-80k samples streamed in 4000-sample chunks (8-20 chunks). The interrupt can abort anywhere between chunks.

### CRITICAL: What Already Works (Do NOT Rewrite)

- `common/src/protocol.rs` — Wire format, all existing message types. Only ADD the new `InterruptTts` variant.
- `server/src/session.rs` — `run_session()`, `stt_router()`, `tts_router()`, `send_tts_audio()`. EXTEND, don't rewrite. The shared writer (`Arc<Mutex<BufWriter<TcpStream>>>`) and pause/resume architecture are already in place.
- `server/src/tts.rs` — `TtsEngine` trait, `KokoroTts`, `set_speed()`. No changes needed.
- `client/src/vad.rs` — `VoiceDetector` struct and `process_samples()`. No changes needed — VAD already works; we just need to run it during playback.
- `client/src/playback.rs` — `start_playback()` function. Might need a channel-based clear mechanism.
- `client/src/audio.rs` — Audio capture and resampling. No changes.

### CRITICAL: Protocol Addition — InterruptTts(0x04)

File: `common/src/protocol.rs`

Add to `ClientMsg` enum (after `ResumeRequest` at line 24):
```rust
pub enum ClientMsg {
    AudioSegment(Vec<i16>), // tag 0x01
    PauseRequest,           // tag 0x02
    ResumeRequest,          // tag 0x03
    InterruptTts,           // tag 0x04, empty payload  ← NEW
}
```

Add write support in `write_client_msg` (after ResumeRequest case, line 78):
```rust
ClientMsg::InterruptTts => {
    w.write_all(&[0x04])?;
    w.write_all(&0u32.to_le_bytes())?;
    w.flush()?;
}
```

Add read support in `read_client_msg` (after 0x03 case, line 118):
```rust
0x04 => {
    if len > 0 {
        let mut discard = vec![0u8; len];
        r.read_exact(&mut discard)?;
    }
    Ok(ClientMsg::InterruptTts)
}
```

Follow the exact pattern of `PauseRequest`/`ResumeRequest` — empty payload, tag in 0x01-0x7F namespace.

### CRITICAL: Server — Interrupt Flag Architecture

File: `server/src/session.rs`

In `run_session()` (line 28), create a shared interrupt flag alongside the existing pause flag:
```rust
let tts_interrupted = Arc::new(AtomicBool::new(false));
let interrupted_stt = tts_interrupted.clone();
let interrupted_tts = tts_interrupted;
```

Pass `interrupted_stt` to `stt_router()` and `interrupted_tts` to `tts_router()`.

**stt_router changes** (line 109): Add `InterruptTts` match arm:
```rust
ClientMsg::InterruptTts => {
    tts_interrupted.store(true, Ordering::SeqCst);
    info!("[server] TTS interrupted by client");
}
```

**tts_router changes** (line 175): At the start of `ResponseText` handling, reset the flag:
```rust
OrchestratorMsg::ResponseText(text) => {
    tts_interrupted.store(false, Ordering::SeqCst);
    // ... existing pause check, speed marker parsing, TTS synthesis ...
}
```

**send_tts_audio changes** (line 287): Add interrupt checking between chunks:
```rust
fn send_tts_audio(
    writer: &mut impl Write,
    samples: &[i16],
    interrupted: &AtomicBool,  // ← NEW parameter
) -> Result<bool> {  // ← Returns true if interrupted
    for chunk in samples.chunks(TTS_CHUNK_SIZE) {
        if interrupted.load(Ordering::SeqCst) {
            info!("[server] TTS streaming interrupted — aborting remaining chunks");
            write_server_msg(writer, &ServerMsg::TtsEnd)?;
            return Ok(true);
        }
        write_server_msg(writer, &ServerMsg::TtsAudioChunk(chunk.to_vec()))?;
    }
    write_server_msg(writer, &ServerMsg::TtsEnd)?;
    Ok(false)
}
```

### CRITICAL: Client — Playback State + Barge-in Detection

File: `client/src/main.rs`

**New state flag** — `is_playing: Arc<AtomicBool>`:
```rust
let is_playing = Arc::new(AtomicBool::new(false));
```

Pass a clone to `tcp_reader_loop`. In `tcp_reader_loop`:
```rust
ServerMsg::TtsAudioChunk(samples) => {
    is_playing.store(true, Ordering::SeqCst);  // ← NEW
    // ... existing resampling + playback_tx.send() ...
}
ServerMsg::TtsEnd => {
    is_playing.store(false, Ordering::SeqCst);  // ← NEW
    debug!("[client] TtsEnd received");
}
```

**Barge-in detection** in main audio loop (around line 160-206). Currently, when `is_listening == true`, VAD runs and segments are sent. For barge-in, add this logic **before** the normal VAD flow:

```rust
// Barge-in: detect voice during TTS playback
const BARGE_IN_FRAMES: u32 = 3; // 30ms sustained voice = barge-in trigger
let mut barge_in_voice_frames: u32 = 0;

// Inside the loop, after resampling:
if listening && is_playing.load(Ordering::SeqCst) {
    // Check for voice activity during playback (barge-in detection)
    // Use raw VAD frames, not full segment detection
    let has_voice = check_voice_frames(&resampled);  // simplified VAD check
    if has_voice {
        barge_in_voice_frames += 1;
    } else {
        barge_in_voice_frames = 0;
    }
    if barge_in_voice_frames >= BARGE_IN_FRAMES {
        info!("[BARGE-IN] Interrupting TTS playback");
        let _ = write_client_msg(&mut writer, &ClientMsg::InterruptTts);
        is_playing.store(false, Ordering::SeqCst);
        // Clear playback buffer by sending sentinel
        let _ = playback_tx.send(vec![]);
        barge_in_voice_frames = 0;
        // Don't continue — fall through to normal VAD to capture the speech
    }
}
```

**Voice frame detection during playback**: Use the existing webrtc-vad instance or create a lightweight check. The simplest approach is to add a `has_voice_activity(&self, samples: &[i16]) -> bool` method to `VoiceDetector` that checks if any 10ms frame in the samples contains voice, WITHOUT accumulating into the segment buffer. This keeps barge-in detection independent from the segment state machine.

Alternatively, count voice frames directly using `vad.is_voice_segment()` on 160-sample frames (same as `process_samples` does internally but without buffering).

### CRITICAL: Playback Buffer Clear

File: `client/src/playback.rs`

The playback thread pulls samples from `playback_rx`. To clear the buffer on barge-in:

**Option A — Drain channel:** After sending InterruptTts, drain all pending messages from playback_tx/rx:
```rust
// In main loop after interrupt:
while playback_tx.try_send(vec![]).is_ok() {} // fill with empties to flush
```
This is fragile. Better:

**Option B — Sentinel value:** When the playback thread receives an empty `Vec<i16>`, it drains the channel and stops:
```rust
// In playback callback or a wrapper:
match audio_rx.try_recv() {
    Ok(samples) if samples.is_empty() => {
        // Sentinel: drain remaining and silence output
        while audio_rx.try_recv().is_ok() {}
        // Fill output with silence
    }
    Ok(samples) => { /* normal playback */ }
    Err(_) => { /* no data, fill silence */ }
}
```

**Option C — AtomicBool clear flag:** Share a `playback_clear` flag. When set, playback thread drains its internal buffer and the channel. Reset after drain. This is simplest:
```rust
if clear_flag.load(Ordering::SeqCst) {
    while audio_rx.try_recv().is_ok() {} // drain
    clear_flag.store(false, Ordering::SeqCst);
    // fill output buffer with zeros
}
```

Option C is recommended — matches existing AtomicBool patterns in the codebase.

### CRITICAL: What NOT to Do

1. **Do NOT add async/tokio.** The codebase uses OS threads + crossbeam-channel everywhere.
2. **Do NOT try to cancel `tts.synthesize()` mid-inference.** ONNX inference is atomic. The interrupt happens between chunk sends, not during synthesis.
3. **Do NOT modify the TtsEngine trait.** Synthesis stays synchronous and blocking. The interrupt is purely at the streaming level.
4. **Do NOT add echo cancellation.** The `BARGE_IN_FRAMES` threshold (30ms sustained voice) is sufficient to filter speaker echo. Full AEC is out of scope.
5. **Do NOT change the hotkey toggle behavior.** Barge-in only activates when `is_listening == true` (hotkey held). This is by design — the hotkey controls the entire conversation flow.
6. **Do NOT modify the orchestrator.** The orchestrator is unaware of barge-in. It simply receives transcribed text and sends responses. The interrupt is transparent to it (just like pause/resume).

### Server — Handling InterruptTts During Active Synthesis

If `InterruptTts` arrives while `tts.synthesize()` is still running (blocking), the `stt_router` sets the flag immediately (it's on a different thread). When `synthesize()` completes and `tts_router` starts streaming chunks via `send_tts_audio()`, it checks the flag on the very first chunk and aborts. This means the synthesized audio is discarded without being sent — correct behavior.

Timeline:
```
stt_router:  ...receives InterruptTts → sets flag...
tts_router:  ...synthesize() still running... → finishes → checks flag → aborts sending
```

### Previous Story Intelligence

From stories 2-3, 2-4, 2-5, 3-1, 3-2, 4-1, 4-2:
- Package naming: `space_lt_*` (underscore in code, hyphen in Cargo.toml)
- ALWAYS use `make check` not raw cargo commands
- Clippy: `-D warnings` — all warnings are errors
- Error handling: `anyhow::Result` + `.context()` — NOT `map_err`
- Logging: `[server]`/`[client]` prefix, `debug!()` for verbose, `info!()` for user-facing
- Test convention: inline `#[cfg(test)]` modules, `match`-based assertions
- Protocol functions flush internally — no explicit `flush()` after write calls
- Shared state uses `Arc<AtomicBool>` — existing pattern from pause/resume
- Shared TCP writer: `Arc<Mutex<BufWriter<TcpStream>>>` — already used by stt_router and tts_router
- Test setup: use `setup_session()` helper for integration tests (creates TCP+Unix connections + mock transcriber/TTS)
- Test text messages: handle `ServerMsg::Text(_)` in match arms (display texts from "You:" and "AI:" prefixes)

### Current Test Counts

90 tests total across workspace: 20 client + 34 common + 16 orchestrator + 20 server.
All must continue passing after this story (no regressions).

### Threading Model Reminder

```
Server threads:
  stt_router:  reads TCP (ClientMsg) → writes Unix (OrchestratorMsg)
               NOW ALSO: handles InterruptTts → sets interrupted flag
  tts_router:  reads Unix (OrchestratorMsg) → writes TCP (ServerMsg)
               NOW ALSO: checks interrupted flag between chunk sends

Client threads:
  main:         audio capture → VAD → AudioSegment sending
                NOW ALSO: barge-in detection during playback
  tcp_reader:   reads TCP → routes to playback channel
                NOW ALSO: sets is_playing flag
  playback:     pulls from channel → cpal output
                NOW ALSO: handles clear signal
  hotkey:       evdev listener → is_listening flag (unchanged)
```

### Project Structure Notes

Files to modify:
- `common/src/protocol.rs` (MODIFY) — Add `InterruptTts` variant + serialization
- `server/src/session.rs` (MODIFY) — Add interrupt flag, handle InterruptTts, modify send_tts_audio
- `client/src/main.rs` (MODIFY) — Add is_playing state, barge-in detection in audio loop
- `client/src/playback.rs` (MODIFY) — Add buffer clear mechanism

Files NOT to modify:
- `server/src/tts.rs` — TtsEngine trait stays unchanged
- `server/src/server.rs` — No changes needed
- `orchestrator/src/*` — Orchestrator is unaware of barge-in
- `client/src/vad.rs` — Existing VoiceDetector works as-is (may add a `has_voice_activity` helper)
- `client/src/audio.rs` — No changes
- `client/src/hotkey.rs` — No changes
- `client/src/connection.rs` — No changes
- `agent/language_trainer.agent.md` — No changes

### References

- [Source: architecture.md#Gap Resolutions G7] — InterruptTts deferred to post-MVP, now being implemented
- [Source: architecture.md#Gap Resolutions G11] — Barge-in handling (MVP: client mutes capture while TTS plays)
- [Source: architecture.md#Audio & Protocol Conventions] — Client→Server tags 0x01-0x7F, available tag 0x04
- [Source: architecture.md#Concurrency & Resource Patterns] — OS threads + crossbeam, AtomicBool shared state
- [Source: epics.md#Story 6.1] — Acceptance criteria and NFRs
- [Source: epics.md#Epic Dependencies] — Story 6.2 depends on 6.1 for interrupt handling
- [Source: server/src/session.rs] — run_session(), stt_router(), tts_router(), send_tts_audio()
- [Source: common/src/protocol.rs] — ClientMsg, ServerMsg, tag namespaces
- [Source: client/src/main.rs] — Main audio loop, tcp_reader_loop, playback setup
- [Source: client/src/vad.rs] — VoiceDetector, FRAME_SIZE=160, SILENCE_THRESHOLD=50
- [Source: client/src/playback.rs] — start_playback(), crossbeam channel consumer

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6 (claude-opus-4-6)

### Debug Log References

- `VoiceDetector::has_voice_activity` initially implemented as `&self` — webrtc-vad `Vad::is_voice_segment` requires `&mut self`, changed to `&mut self`
- Playback clear mechanism: chose AtomicBool flag (Option C from story Dev Notes) over sentinel value — cleaner, matches existing codebase patterns
- Barge-in detection uses `BARGE_IN_THRESHOLD = 3` consecutive audio chunks with voice activity (not 3 VAD frames of 10ms — each audio chunk contains multiple frames)

### Completion Notes List

- Added `InterruptTts(0x04)` to `ClientMsg` enum with write/read support + round-trip test + multi-message stream test (35 common tests, +1)
- Server: shared `tts_interrupted: Arc<AtomicBool>` between stt_router (sets flag) and tts_router (checks between chunks). `send_tts_audio()` now returns `bool` indicating interruption and accepts interrupt flag parameter.
- Server: 2 new integration tests — `interrupt_tts_aborts_audio_stream` (verifies <5 chunks sent) and `interrupt_tts_normal_flow_without_interrupt` (verifies all 5 chunks sent). 23 server tests total (+2).
- Client: `is_playing` AtomicBool tracked in tcp_reader_loop (set on TtsAudioChunk, cleared on TtsEnd)
- Client: `playback_clear` AtomicBool shared with playback callback — on barge-in, drains channel + leftover buffer + fills silence
- Client: barge-in detection in main audio loop — when `is_listening && is_playing`, counts consecutive chunks with voice activity via `VoiceDetector::has_voice_activity()`. Triggers at 3 consecutive chunks → sends InterruptTts, clears playback, resets counter.
- Client: after barge-in, VAD continues normally to collect user speech → AudioSegment sent to server as new turn
- All 93 tests pass (21 client + 35 common + 14 orchestrator + 23 server). `make check` clean (fmt + clippy + tests).
- Code review fixes: separated barge-in VAD instance from segment accumulation VAD (M1), added 3 unit tests for `has_voice_activity` (M2), added 3 deterministic unit tests for `send_tts_audio` (M3). All 99 tests pass (24 client + 35 common + 14 orchestrator + 26 server).

### File List

- common/src/protocol.rs (MODIFIED) — Added `InterruptTts` variant to `ClientMsg`, write/read for tag 0x04, round-trip test, multi-message stream test
- server/src/session.rs (MODIFIED) — Added `tts_interrupted` shared flag, `InterruptTts` handling in stt_router, interrupt check in send_tts_audio, reset in tts_router, 2 new integration tests
- client/src/main.rs (MODIFIED) — Added `is_playing` + `playback_clear` flags, barge-in detection loop with BARGE_IN_THRESHOLD, is_playing tracking in tcp_reader_loop
- client/src/playback.rs (MODIFIED) — Added `clear: Arc<AtomicBool>` parameter to start_playback, clear logic in callback
- client/src/vad.rs (MODIFIED) — Added `has_voice_activity(&mut self, samples)` method for barge-in detection

### Change Log

- 2026-02-21: Implemented barge-in interruption (Story 6.1) — InterruptTts(0x04) protocol message, server interrupt flag with chunk-level abort, client playback state tracking, barge-in voice detection with 3-chunk threshold, playback buffer clear mechanism. All 93 tests passing.
- 2026-02-21: Code review fixes — separated barge-in VAD (M1: double state mutation), added has_voice_activity tests (M2), added deterministic send_tts_audio unit tests (M3). All 99 tests passing.
