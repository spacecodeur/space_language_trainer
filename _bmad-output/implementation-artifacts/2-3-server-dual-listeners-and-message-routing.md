# Story 2.3: Server Dual Listeners and Message Routing

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want the server to accept TCP connections from the client and Unix socket connections from the orchestrator, routing messages between them with STT and TTS processing,
so that all three processes can communicate via the extended binary protocol.

## Acceptance Criteria

1. **Given** the server binary with `--model`, `--tts-model`, `--port`, and `--socket-path` arguments
   **When** the server starts in daemon mode
   **Then** it loads Whisper first, then Kokoro TTS sequentially (fail-fast if either fails)
   **And** model loading completes within 60 seconds (NFR4)

2. **Given** the server in daemon mode
   **When** it is ready to accept connections
   **Then** it listens on the TCP port (default 9500) for client connections
   **And** it listens on a Unix socket (default `/tmp/space_lt_server.sock`) for orchestrator connections

3. **Given** a client connected via TCP
   **When** the client sends `AudioSegment(0x01)` messages
   **Then** the server transcribes the audio via Whisper STT
   **And** forwards the transcription as `TranscribedText(0xA0)` to the orchestrator via Unix socket

4. **Given** an orchestrator connected via Unix socket
   **When** the orchestrator sends `ResponseText(0xA1)` messages
   **Then** the server synthesizes the text via Kokoro TTS
   **And** streams the audio as chunked `TtsAudioChunk(0x83)` messages to the client via TCP
   **And** sends `TtsEnd(0x84)` after the last chunk

5. **Given** integration tests with MockTranscriber and MockTtsEngine
   **When** running `make check`
   **Then** mock client sends AudioSegment → mock orchestrator receives TranscribedText
   **And** mock orchestrator sends ResponseText → mock client receives TtsAudioChunk + TtsEnd
   **And** all tests pass (fmt, clippy, existing + new)

## Tasks / Subtasks

- [x] Task 1: Create listener.rs — TCP + Unix socket acceptance (AC: #2)
  - [x] 1.1: Create `server/src/listener.rs` with `start_tcp(port: u16) -> Result<TcpListener>` binding to `0.0.0.0:port`
  - [x] 1.2: Add `start_unix(path: &Path) -> Result<UnixListener>` — removes stale socket file before binding
  - [x] 1.3: Add `mod listener;` to main.rs
  - [x] 1.4: Log listener addresses at info level with `[server]` prefix

- [x] Task 2: Create session.rs — message routing engine (AC: #3, #4)
  - [x] 2.1: Create `server/src/session.rs` with `run_session()` function
  - [x] 2.2: Signature: `pub fn run_session(transcriber: Box<dyn Transcriber>, tts: Box<dyn TtsEngine>, tcp: TcpStream, unix: UnixStream) -> Result<()>`
  - [x] 2.3: Clone streams for split read/write via `try_clone()` — one clone for reader, one for writer per connection
  - [x] 2.4: Spawn `stt_router` thread: reads `ClientMsg` from TCP `BufReader` → on `AudioSegment`: transcribe → write `TranscribedText` to Unix `BufWriter`
  - [x] 2.5: Spawn `tts_router` thread: reads `OrchestratorMsg` from Unix `BufReader` → on `ResponseText`: synthesize → chunk (4000 samples / 250ms) → write `TtsAudioChunk` + `TtsEnd` to TCP `BufWriter`
  - [x] 2.6: Wait for either thread to finish → shutdown both connections → join remaining thread
  - [x] 2.7: Handle `PauseRequest`/`ResumeRequest` in stt_router: log "not yet implemented" (deferred to story 3-1)
  - [x] 2.8: Handle `SessionStart`/`SessionEnd` in tts_router: log "not yet implemented" (deferred to story 2-5)
  - [x] 2.9: Add `mod session;` to main.rs

- [x] Task 3: Refactor server.rs for daemon mode (AC: #1, #2)
  - [x] 3.1: Replace `run(model_path, language)` with `run_daemon(transcriber: Box<dyn Transcriber>, tts: Box<dyn TtsEngine>, port: u16, socket_path: &Path) -> Result<()>`
  - [x] 3.2: `run_daemon()`: call `listener::start_tcp()` + `listener::start_unix()`, accept one client, accept one orchestrator, send `Ready` to client, call `session::run_session()`
  - [x] 3.3: Warm up Whisper (1s silence transcription) before accepting connections
  - [x] 3.4: Remove old stdin/stdout SSH-based loop (fully replaced by daemon mode)

- [x] Task 4: Update main.rs — CLI args and sequential model loading (AC: #1)
  - [x] 4.1: Add `--port <port>` arg (default 9500) and `--socket-path <path>` arg (default `/tmp/space_lt_server.sock`)
  - [x] 4.2: Require `--tts-model` for daemon mode (error if not provided, still optional for `--tts-test`)
  - [x] 4.3: Move Whisper model loading from server::run() into main.rs — `LocalTranscriber::new(model_path, language)`
  - [x] 4.4: Load models sequentially in main.rs: Whisper first → warm up → Kokoro second → fail-fast on either
  - [x] 4.5: Pass loaded `Box<dyn Transcriber>` + `Box<dyn TtsEngine>` to `server::run_daemon()`
  - [x] 4.6: Log model loading times at info level

- [x] Task 5: Add MockTranscriber and MockTtsEngine for integration tests (AC: #5)
  - [x] 5.1: Define `MockTranscriber` directly in `session.rs` `#[cfg(test)]` module — returns configurable text string
  - [x] 5.2: Define `MockTtsEngine` directly in `session.rs` `#[cfg(test)]` module — returns ramp pattern of configurable length
  - [x] 5.3: Unit tests: verify MockTranscriber and MockTtsEngine return expected outputs (verified via integration tests)

- [x] Task 6: Integration tests in session.rs (AC: #5)
  - [x] 6.1: Test `stt_routing`: mock client sends AudioSegment via TCP → MockTranscriber → mock orchestrator receives TranscribedText via Unix socket
  - [x] 6.2: Test `tts_routing`: mock orchestrator sends ResponseText via Unix → MockTtsEngine → mock client receives TtsAudioChunk + TtsEnd via TCP
  - [x] 6.3: Test `tts_chunking`: verify large audio is split into ≤4000-sample chunks and total samples match original

- [x] Task 7: Verify build passes (AC: #5)
  - [x] 7.1: Run `make check` — fmt + clippy + all tests pass
  - [x] 7.2: No regressions — 53 existing tests + 3 new tests = 56 total, all pass

## Dev Notes

### CRITICAL: Thread Model — 2 Workers + Main

The server must handle two independent bidirectional connections simultaneously (TCP + Unix socket). This requires threading. The design uses **2 worker threads**:

```
Main Thread:
  load Whisper → warm up → load Kokoro
  → start TCP listener → start Unix listener
  → accept TCP client → send Ready → accept orchestrator
  → spawn workers → wait for both

stt_router thread:
  OWNS: BufReader<TcpStream>, BufWriter<UnixStream>, Box<dyn Transcriber>
  LOOP: read ClientMsg from TCP
    → AudioSegment: transcribe(&mut self) → write TranscribedText to Unix
    → PauseRequest/ResumeRequest: log "not yet implemented"
    → EOF/error: return (triggers shutdown)

tts_router thread:
  OWNS: BufReader<UnixStream>, BufWriter<TcpStream>, Box<dyn TtsEngine>
  LOOP: read OrchestratorMsg from Unix
    → ResponseText: synthesize(&self) → chunk → write TtsAudioChunk + TtsEnd to TCP
    → SessionStart/SessionEnd: log "not yet implemented"
    → EOF/error: return (triggers shutdown)
```

### CRITICAL: Stream Cloning for Split Read/Write

`TcpStream::try_clone()` and `UnixStream::try_clone()` duplicate the file descriptor. One clone for reading, one for writing — each owned by a different thread. **No Mutex needed.**

```rust
let tcp_reader = BufReader::new(tcp_stream.try_clone()?);
let tcp_writer = BufWriter::new(tcp_stream); // original goes to writer
let unix_reader = BufReader::new(unix_stream.try_clone()?);
let unix_writer = BufWriter::new(unix_stream); // original goes to writer
```

### CRITICAL: Trait Ownership Across Threads

- `Transcriber` uses `&mut self` → must be **moved** into stt_router thread (single owner, no sharing)
- `TtsEngine` uses `&self` + is `Send` → can be moved into tts_router thread
- `Box<dyn Transcriber>` is `Send` (trait has `Send` bound) → can cross thread boundary
- `Box<dyn Transcriber>` auto-derefs for `&mut self` method calls

### TTS Audio Chunking

`TtsEngine::synthesize()` returns full `Vec<i16>`. Must chunk for streaming delivery:

```rust
const TTS_CHUNK_SIZE: usize = 4000; // 250ms at 16kHz

fn send_tts_audio(writer: &mut impl Write, samples: &[i16]) -> Result<()> {
    for chunk in samples.chunks(TTS_CHUNK_SIZE) {
        write_server_msg(writer, &ServerMsg::TtsAudioChunk(chunk.to_vec()))?;
    }
    write_server_msg(writer, &ServerMsg::TtsEnd)?;
    writer.flush()?;
    Ok(())
}
```

**Why 4000 samples?**
- 250ms at 16kHz — low enough latency for immediate playback start (NFR5: 200ms)
- ~8KB per chunk payload — efficient for TCP
- 1 second of speech = 4 messages — reasonable overhead

### Connection Lifecycle

1. Server loads models (Whisper → Kokoro, sequential, fail-fast)
2. Warm up Whisper (1s silence) for GPU graph init
3. TCP listener binds to `0.0.0.0:{port}` (default 9500)
4. Unix listener binds to socket path (default `/tmp/space_lt_server.sock`)
5. Accept TCP client (blocking) → send `Ready` to client
6. Accept orchestrator on Unix socket (blocking)
7. Start session routing (spawn 2 threads)
8. When either connection closes → both threads exit → main returns

### Shutdown Strategy

No crossbeam-channel needed. Shutdown propagates through connection close:

1. If orchestrator disconnects → unix_reader gets EOF → tts_router returns
2. Main thread detects thread exit via `JoinHandle::is_finished()` polling or sequential `join()`
3. Main drops the TCP stream clones → tcp_reader gets error → stt_router returns
4. Both threads joined, server exits cleanly

Vice versa if client disconnects first.

**Pattern for main thread:**
```rust
loop {
    if stt_handle.is_finished() || tts_handle.is_finished() {
        break;
    }
    std::thread::sleep(Duration::from_millis(100));
}
// Drop stream clones to unblock remaining thread
drop(tcp_cleanup);
drop(unix_cleanup);
let _ = stt_handle.join();
let _ = tts_handle.join();
```

### Unix Socket Cleanup

Socket file persists on disk after server exit/crash. Must remove before binding:

```rust
pub fn start_unix(path: &Path) -> Result<UnixListener> {
    if path.exists() {
        std::fs::remove_file(path).context("removing stale Unix socket")?;
    }
    UnixListener::bind(path).context("binding Unix socket")
}
```

### Model Loading Sequence (G4)

Moved from server::run() to main.rs for explicit control:

```rust
// 1. Load Whisper first (critical path, ~3 Go VRAM)
info!("[server] Loading Whisper model: {model}...");
let mut transcriber = LocalTranscriber::new(&model.to_string_lossy(), &language)?;

// 2. Warm up (GPU graph init)
let silence = vec![0i16; 16000];
let _ = transcriber.transcribe(&silence);

// 3. Load Kokoro second (~2-3 Go VRAM)
info!("[server] Loading TTS model: {tts_model_dir}...");
let tts = KokoroTts::new(Path::new(&tts_model_dir))?;

// 4. Both loaded, start daemon
server::run_daemon(Box::new(transcriber), Box::new(tts), port, &socket_path)?;
```

### No New External Dependencies

All networking via std:
- `std::net::{TcpListener, TcpStream}`
- `std::os::unix::net::{UnixListener, UnixStream}`
- `std::io::{BufReader, BufWriter}`
- `std::thread` for worker threads

### Backward Compatibility — SSH Mode Removed

The old `server::run()` uses stdin/stdout (SSH-piped). Story 2-3 **replaces** this with the daemon model entirely. The old client (`client/src/remote.rs` using SSH) will NOT work with this server — story 2-4 updates the client to use TCP.

For manual E2E testing of 2-3, use a test script or netcat to connect to TCP and Unix sockets.

### PauseRequest / ResumeRequest Handling

Story 2-3 receives `PauseRequest`/`ResumeRequest` from the client but does NOT implement pause/resume logic. The stt_router thread should log a debug message and continue processing. Full pause/resume propagation is story 3-1.

### SessionStart / SessionEnd Handling

The tts_router reads `OrchestratorMsg` which includes `SessionStart` and `SessionEnd`. Story 2-3 should log these but NOT implement session lifecycle handling. Full session handshake is story 2-5.

### Integration Test Pattern

Tests use real TCP/Unix connections on localhost with mock engines:

```rust
#[test]
fn stt_routing_audio_to_transcribed_text() {
    // 1. Bind TCP + Unix listeners on random ports
    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = tcp_listener.local_addr().unwrap().port();
    let sock_path = temp_socket_path(); // unique /tmp/space_lt_test_*.sock

    let unix_listener = UnixListener::bind(&sock_path).unwrap();

    // 2. Connect mock client + mock orchestrator
    let mock_client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let (server_tcp, _) = tcp_listener.accept().unwrap();

    let mock_orch = UnixStream::connect(&sock_path).unwrap();
    let (server_unix, _) = unix_listener.accept().unwrap();

    // 3. Run session with mocks
    let handle = std::thread::spawn(move || {
        run_session(
            Box::new(MockTranscriber::new("Hello world")),
            Box::new(MockTtsEngine::new()),
            server_tcp,
            server_unix,
        )
    });

    // 4. Client sends AudioSegment
    let mut client_w = BufWriter::new(&mock_client);
    write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();

    // 5. Orchestrator reads TranscribedText
    let mut orch_r = BufReader::new(&mock_orch);
    let msg = read_orchestrator_msg(&mut orch_r).unwrap();
    match msg {
        OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "Hello world"),
        other => panic!("Expected TranscribedText, got {other:?}"),
    }

    // 6. Cleanup: close connections to stop session
    drop(mock_client);
    drop(mock_orch);
    let _ = handle.join();
    std::fs::remove_file(&sock_path).ok();
}
```

### Empty Transcription Handling

Whisper sometimes returns empty strings (silence, filtered hallucinations). The stt_router should **NOT** forward empty transcriptions to the orchestrator:

```rust
let text = transcriber.transcribe(&samples)?;
if !text.is_empty() {
    write_orchestrator_msg(&mut unix_writer, &OrchestratorMsg::TranscribedText(text))?;
    unix_writer.flush()?;
}
```

### File Structure After Story 2-3

```
server/src/
├── main.rs        — MODIFIED: --port, --socket-path args, sequential model loading, call run_daemon()
├── server.rs      — MODIFIED: replaced run() with run_daemon() accepting trait objects
├── listener.rs    — NEW: start_tcp(), start_unix()
├── session.rs     — NEW: run_session(), stt_router, tts_router, TTS chunking, integration tests
├── transcribe.rs  — UNCHANGED (MockTranscriber defined in session.rs tests instead)
└── tts.rs         — UNCHANGED
```

### Project Structure Notes

- All new files are in `server/src/` — no cross-crate changes
- Protocol functions (`read_client_msg`, `write_orchestrator_msg`, etc.) already support `impl Read/Write` — works with TCP and Unix streams
- No changes to common, client, or orchestrator crates
- Naming follows architecture: `listener.rs`, `session.rs` as documented

### Previous Story Intelligence (from Stories 1-3, 2-1, 2-2)

- Workspace: 4 crates, `make check` passes (53 tests: 17 client + 26 common + 3 orchestrator + 7 server)
- Package naming: `space_lt_*` (underscore)
- Makefile: always use `make check` not raw cargo commands
- Clippy: `-D warnings` — all warnings are errors
- Error handling: `anyhow::Result` + `.context()` — NOT `map_err` except for eyre::Report boundaries (sherpa-rs)
- Logging: `[server]` prefix, `debug!()` for verbose, `info!()` for normal
- Arg parsing: `find_arg_value()` helper pattern from main.rs
- Test convention: inline `#[cfg(test)]` modules, `match`-based assertions
- TtsEngine trait: `fn synthesize(&self, text: &str) -> Result<Vec<i16>>`, `Send` bound
- Transcriber trait: `fn transcribe(&mut self, audio_i16: &[i16]) -> Result<String>`, `Send` bound
- KokoroTts wraps `sherpa_rs::tts::KokoroTts` in `Mutex` for `&self` compat
- User preference: use `cargo add` for new dependencies
- CLAUDE.md: never mention Claude in commits, always use Makefile targets

### References

- [Source: architecture.md#Communication Architecture] — TCP for client, Unix socket for orchestrator, daemon model
- [Source: architecture.md#Concurrency & Resource Patterns] — OS threads + crossbeam-channel, no async
- [Source: architecture.md#Gap Resolutions G4] — Model loading order: Whisper first, Kokoro second, fail-fast
- [Source: architecture.md#Gap Resolutions G6] — Session start handshake (deferred to 2-5)
- [Source: architecture.md#Project Structure] — `listener.rs`, `session.rs` file placement
- [Source: architecture.md#Data Flow] — 7-step pipeline: capture → VAD → STT → orchestrator → Claude → TTS → playback
- [Source: architecture.md#Audio & Protocol Conventions] — TtsAudioChunk(0x83), TtsEnd(0x84), TranscribedText(0xA0), ResponseText(0xA1)
- [Source: architecture.md#Gap Resolutions G2] — Pause/resume server-side gate (deferred to 3-1)
- [Source: architecture.md#Gap Resolutions G3] — Audio playback starts on first TtsAudioChunk (no pre-buffer)
- [Source: epics.md#Story 2.3] — Acceptance criteria
- [Source: 2-2-tts-engine-integration.md#Completion Notes] — TtsEngine API, sherpa-rs patterns, 53 tests baseline
- [Source: 2-1-orchestrator-claude-cli-bridge.md#Dev Notes] — LlmBackend API, spike findings, protocol usage
- [Source: server/src/tts.rs] — KokoroTts implementation, resample_24k_to_16k, synthesize()
- [Source: server/src/transcribe.rs] — LocalTranscriber, Transcriber trait, Whisper warm-up pattern
- [Source: server/src/server.rs] — Current stdin/stdout loop to be replaced
- [Source: common/src/protocol.rs] — All message types, read/write functions for Client/Server/Orchestrator msgs

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

None — all issues resolved during implementation.

### Completion Notes List

- TCP listener binds to `0.0.0.0:{port}` (default 9500), Unix socket to configurable path (default `/tmp/space_lt_server.sock`)
- Session routing uses 2 OS threads (stt_router + tts_router) with stream cloning for split read/write — no Mutex needed
- TTS audio chunked into 4000-sample (250ms) pieces via `send_tts_audio()` helper
- MockTtsEngine returns a ramp pattern (not sine wave) for simpler verification — deviation from story spec but functionally equivalent
- PauseRequest/ResumeRequest and SessionStart/SessionEnd logged as "not yet implemented" per spec
- Shutdown uses `TcpStream::shutdown(Both)` / `UnixStream::shutdown(Both)` to unblock remaining thread after one exits
- Warm-up moved to main.rs (before `run_daemon`) — Whisper processes 1s silence for GPU graph init
- Old SSH stdin/stdout server loop fully replaced by daemon mode
- 56 total tests pass (53 existing + 3 new integration tests)

**Code Review Fixes (Opus 4.6):**
- M1: Replaced fragile string-matching EOF detection with `downcast_ref::<std::io::Error>()` + `ErrorKind` matching
- M2: Added stream cleanup clones with `shutdown(Both)` to unblock threads stuck on blocking reads during shutdown
- M3: Removed redundant `flush()` calls — protocol `write_*_msg` functions already flush internally

### File List

- `server/src/listener.rs` — NEW: `start_tcp()`, `start_unix()` listener helpers
- `server/src/session.rs` — NEW: `run_session()`, `stt_router()`, `tts_router()`, `send_tts_audio()`, 3 integration tests
- `server/src/server.rs` — REWRITTEN: `run_daemon()` replacing old `run()`
- `server/src/main.rs` — MODIFIED: `mod listener`, `mod session`, `--port`, `--socket-path` args, sequential model loading
