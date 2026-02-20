# Story 1.3: Extend Binary Protocol with New Message Types

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want to extend the binary protocol with all 8 new message types for TTS, pause/resume, and orchestrator communication,
so that the protocol foundation is ready for all future epics.

## Acceptance Criteria

1. **Given** the existing protocol in `common/src/protocol.rs` with `AudioSegment(0x01)`, `Ready(0x80)`, `Text(0x81)`, `Error(0x82)`
   **When** the developer adds the 8 new message types
   **Then** the following messages are implemented: `PauseRequest(0x02)`, `ResumeRequest(0x03)`, `TtsAudioChunk(0x83)`, `TtsEnd(0x84)`, `TranscribedText(0xA0)`, `ResponseText(0xA1)`, `SessionStart(0xA2)`, `SessionEnd(0xA3)`

2. **Given** the new message types
   **When** examining the tag values
   **Then** tag namespaces are respected: client→server `0x01-0x7F`, server→client `0x80-0xFF`, orchestrator↔server `0xA0-0xBF`

3. **Given** each new message type
   **When** encoding then decoding the message
   **Then** each has a round-trip serialization unit test (encode → decode → verify equality)

4. **Given** `SessionStart(0xA2)`
   **When** examining its payload
   **Then** it carries a UTF-8 JSON string (the actual JSON structure is NOT parsed at the protocol level — it is a raw String)

5. **Given** all changes
   **When** running `make check`
   **Then** fmt, clippy, and all tests pass (including the 9 existing tests + new tests)

## Tasks / Subtasks

- [x] Task 1: Add PauseRequest and ResumeRequest to ClientMsg (AC: #1, #2, #3)
  - [x] 1.1: Add `PauseRequest` variant (tag 0x02, empty payload) to `ClientMsg` enum
  - [x] 1.2: Add `ResumeRequest` variant (tag 0x03, empty payload) to `ClientMsg` enum
  - [x] 1.3: Add `#[derive(Debug)]` to `ClientMsg` (currently missing — `ServerMsg` has it)
  - [x] 1.4: Extend `write_client_msg` with match arms for both new variants
  - [x] 1.5: Extend `read_client_msg` with tag 0x02 and 0x03 arms
  - [x] 1.6: Add `round_trip_pause_request` test
  - [x] 1.7: Add `round_trip_resume_request` test

- [x] Task 2: Add TtsAudioChunk and TtsEnd to ServerMsg (AC: #1, #2, #3)
  - [x] 2.1: Add `TtsAudioChunk(Vec<i16>)` variant (tag 0x83, payload = raw i16 LE bytes) to `ServerMsg`
  - [x] 2.2: Add `TtsEnd` variant (tag 0x84, empty payload) to `ServerMsg`
  - [x] 2.3: Extend `write_server_msg` with match arms for both new variants
  - [x] 2.4: Extend `read_server_msg` with tag 0x83 and 0x84 arms
  - [x] 2.5: Add `round_trip_tts_audio_chunk` test (with sample data like AudioSegment test)
  - [x] 2.6: Add `round_trip_tts_audio_chunk_empty` test
  - [x] 2.7: Add `round_trip_tts_end` test

- [x] Task 3: Create OrchestratorMsg enum and read/write functions (AC: #1, #2, #3, #4)
  - [x] 3.1: Create `OrchestratorMsg` enum with `#[derive(Debug)]` and 4 variants:
    - `TranscribedText(String)` — tag 0xA0, UTF-8 payload
    - `ResponseText(String)` — tag 0xA1, UTF-8 payload
    - `SessionStart(String)` — tag 0xA2, UTF-8 JSON payload (raw string)
    - `SessionEnd` — tag 0xA3, empty payload
  - [x] 3.2: Create `write_orchestrator_msg(w: &mut impl Write, msg: &OrchestratorMsg) -> Result<()>`
  - [x] 3.3: Create `read_orchestrator_msg(r: &mut impl Read) -> Result<OrchestratorMsg>`
  - [x] 3.4: Add `round_trip_transcribed_text` test
  - [x] 3.5: Add `round_trip_response_text` test
  - [x] 3.6: Add `round_trip_session_start` test (with sample JSON string payload)
  - [x] 3.7: Add `round_trip_session_end` test
  - [x] 3.8: Add `unknown_orchestrator_tag_errors` test

- [x] Task 4: Add multi-message stream tests (AC: #3)
  - [x] 4.1: Add `multiple_client_messages_in_stream` test (AudioSegment + PauseRequest + ResumeRequest)
  - [x] 4.2: Add `multiple_server_messages_with_tts_in_stream` test (Ready + TtsAudioChunk + TtsEnd)
  - [x] 4.3: Add `multiple_orchestrator_messages_in_stream` test (SessionStart + TranscribedText + ResponseText + SessionEnd)

- [x] Task 5: Verify build passes (AC: #5)
  - [x] 5.1: Run `make check` — fmt + clippy + all tests must pass
  - [x] 5.2: Verify all 9 existing tests still pass (no regressions)

## Dev Notes

### Enum Design: Three Separate Enums

The protocol uses **three separate enums** for the three communication channels:

| Enum | Channel | Tags | Direction |
|------|---------|------|-----------|
| `ClientMsg` | TCP | 0x01-0x7F | Client → Server |
| `ServerMsg` | TCP | 0x80-0xFF | Server → Client |
| `OrchestratorMsg` | Unix socket | 0xA0-0xBF | Bidirectional (Orchestrator ↔ Server) |

`OrchestratorMsg` is a **single bidirectional enum**. Both the server and orchestrator read and write it. The tag uniquely identifies the message type; the caller decides which variants to send. This is the simplest approach and consistent with the existing pattern where readers handle all tags.

### Payload Patterns

Follow the **exact patterns** already established in the existing code:

| Payload Type | Pattern | Example |
|-------------|---------|---------|
| Empty | Write tag + 0u32 length | `Ready`, `PauseRequest`, `ResumeRequest`, `TtsEnd`, `SessionEnd` |
| i16 audio | Write tag + (len*2) as u32 + raw i16 LE bytes | `AudioSegment`, `TtsAudioChunk` |
| UTF-8 string | Write tag + bytes.len() as u32 + UTF-8 bytes | `Text`, `Error`, `TranscribedText`, `ResponseText`, `SessionStart` |

### CRITICAL: SessionStart Payload is Raw String

`SessionStart(0xA2)` carries a UTF-8 JSON config string. At the **protocol level**, this is just a `String`. Do **NOT** add `serde`/`serde_json` dependencies. Do **NOT** define a config struct in `protocol.rs`. The JSON parsing happens in higher-level code (orchestrator/server) in future stories.

Test with a sample JSON string like: `{"agent_path": "/path/to/agent.md", "session_dir": "/tmp/session"}`

### TtsAudioChunk Follows AudioSegment Pattern

`TtsAudioChunk(Vec<i16>)` uses the **exact same serialization** as `AudioSegment(Vec<i16>)`:
- Payload = raw i16 samples in little-endian
- Length field = `samples.len() * 2` (i16 = 2 bytes)
- Read: check `len.is_multiple_of(2)`, then decode chunks_exact(2)

Copy-paste the AudioSegment pattern. Do NOT innovate on the wire format.

### Empty Payload Read Pattern

For tags with empty payload, follow the existing `Ready` pattern in `read_server_msg`:
```rust
0x80 => {
    if len > 0 {
        let mut discard = vec![0u8; len];
        r.read_exact(&mut discard)?;
    }
    Ok(ServerMsg::Ready)
}
```
This discards any unexpected payload bytes for forward compatibility. Use this same pattern for all empty-payload messages.

### Derive Debug on ClientMsg

`ClientMsg` is currently missing `#[derive(Debug)]` while `ServerMsg` has it. Add `#[derive(Debug)]` to `ClientMsg` for consistency. Add it to `OrchestratorMsg` as well.

### Test Naming Convention

Follow the existing naming pattern:
- `round_trip_<variant_name_in_snake_case>` — e.g., `round_trip_pause_request`
- `round_trip_<variant>_empty` — for empty-data edge cases
- `multiple_<channel>_messages_in_stream` — for multi-message tests
- `unknown_<channel>_tag_errors` — for unknown tag rejection

### No New Dependencies

This story requires **zero new Cargo dependencies**. Everything is done with `std::io::{Read, Write}` and `anyhow`.

### File Scope

Only one file is modified: **`common/src/protocol.rs`**. No changes to `lib.rs`, no changes to other crates. The existing `write_client_msg`/`read_client_msg` and `write_server_msg`/`read_server_msg` functions are extended in place. New `write_orchestrator_msg`/`read_orchestrator_msg` functions are added.

### Previous Story Intelligence (from Story 1-1 and 1-2)

- Workspace has 4 crates, `make check` passes (33 tests)
- Package naming: `space_lt_*` (underscore)
- Makefile exists — always use `make check` not raw cargo commands
- `.gitignore` excludes `/target`, `/.claude/`, `/spike/results/`
- Story 1-2 (Phase 0 spike) was pure shell — no Rust changes
- Clippy runs with `-D warnings` — all warnings are errors
- Existing tests use `match` patterns (no `PartialEq` on enums)

### References

- [Source: architecture.md#Audio & Protocol Conventions] — Tag namespaces, new message types table
- [Source: architecture.md#Communication Architecture] — TCP + Unix socket channels
- [Source: architecture.md#Core Architectural Decisions] — Protocol extension mentioned
- [Source: epics.md#Story 1.3] — Acceptance criteria
- [Source: architecture.md#Test Organization] — Inline unit tests, pragmatic coverage
- [Source: architecture.md#Enforcement Guidelines] — Round-trip test for every new message type

## Dev Agent Record

### Agent Model Used
claude-opus-4-6

### Debug Log References
None

### Completion Notes List
- All 8 new message types implemented across 3 enums (ClientMsg, ServerMsg, OrchestratorMsg)
- Added `#[derive(Debug)]` to ClientMsg for consistency with ServerMsg
- OrchestratorMsg created as single bidirectional enum with read/write functions
- SessionStart carries raw UTF-8 string (no serde dependency added)
- TtsAudioChunk follows exact AudioSegment serialization pattern
- Empty payload variants use forward-compatible discard pattern
- 15 new tests added (8 round-trip, 3 multi-message stream, 1 unknown tag, 3 edge cases)
- Code review fixes: added 2 empty-string edge case tests, separated PauseRequest/ResumeRequest debug logs
- Fixed exhaustive match errors in server/src/server.rs and client/src/remote.rs for new variants
- `make check` passes: 46 tests total (24 common, 17 client, 5 server), fmt clean, clippy clean
- Zero new Cargo dependencies

### File List
- `common/src/protocol.rs` — Extended ClientMsg/ServerMsg, added OrchestratorMsg enum + read/write functions + 15 new tests
- `server/src/server.rs` — Added match arms for PauseRequest/ResumeRequest (placeholder)
- `client/src/remote.rs` — Added match arms for TtsAudioChunk/TtsEnd (error on unexpected)
