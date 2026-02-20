# Story 2.4: Client TCP Connection and Audio Playback

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want the client to connect to the server via TCP and play received TTS audio through the tablet speakers,
so that the user can hear Claude's spoken responses.

## Acceptance Criteria

1. **Given** the existing client with audio capture, VAD, and resampling
   **When** the developer replaces SSH communication with TCP
   **Then** client connects to server via TCP at specified IP:port (replaces `remote.rs`/SSH)
   **And** existing audio capture and VAD continue to work over TCP (AudioSegment sent via TCP)

2. **Given** a connected client receiving server messages
   **When** `TtsAudioChunk(0x83)` messages arrive from the server
   **Then** they are fed to a cpal output stream for playback through tablet speakers
   **And** playback starts within 200ms of first TtsAudioChunk received (NFR5)

3. **Given** a ring buffer feeding the cpal audio callback
   **When** TTS audio chunks arrive at variable rates
   **Then** the ring buffer handles timing differences between network arrival and audio playback
   **And** `TtsEnd(0x84)` signals end of current TTS response

4. **Given** integration tests with mock server
   **When** running `make check`
   **Then** mock server sends TtsAudioChunk sequence, client receives and decodes correctly
   **And** all existing 56 tests continue to pass (no regressions)

## Tasks / Subtasks

- [x] Task 1: Create connection.rs — TCP client replacing SSH (AC: #1)
  - [x] 1.1: Create `client/src/connection.rs` with `TcpConnection` struct holding `BufReader<TcpStream>` and `BufWriter<TcpStream>` (via `try_clone()`)
  - [x] 1.2: `TcpConnection::connect(addr: &str) -> Result<Self>` — connects to server, waits for `ServerMsg::Ready`
  - [x] 1.3: `send_audio` removed — writer half used directly via `write_client_msg` after `into_split()`
  - [x] 1.4: `TcpConnection::read_server_msg(&mut self) -> Result<ServerMsg>` — reads next message from server
  - [x] 1.5: Add `mod connection;` to main.rs, removed `mod remote;` (combined with Task 6)
  - [x] 1.6: Add disconnect detection via `is_disconnect()` helper (same pattern as server session.rs)

- [x] Task 2: Create playback.rs — cpal audio output stream (AC: #2, #3)
  - [x] 2.1: Create `client/src/playback.rs` with `start_playback()` function returning `(cpal::Stream, u32)`
  - [x] 2.2: Use `crossbeam_channel::Receiver<Vec<i16>>` as audio source for the cpal output callback
  - [x] 2.3: cpal output callback: pop samples from receiver, write to output buffer, fill with silence if no data; leftover buffer for partial chunks
  - [x] 2.4: Configure output stream as 16kHz mono i16 if supported, otherwise native rate
  - [x] 2.5: If device doesn't support 16kHz, resampling done in tcp_reader thread using audio.rs infrastructure
  - [x] 2.6: Log playback device name and actual sample rate at info level

- [x] Task 3: Integrate TCP reader thread for ServerMsg handling (AC: #2, #3)
  - [x] 3.1: In main.rs, spawn a `tcp_reader` thread via `tcp_reader_loop()` using `read_server_msg` on BufReader
  - [x] 3.2: On `ServerMsg::TtsAudioChunk(samples)` — resample if needed, send to playback channel
  - [x] 3.3: On `ServerMsg::TtsEnd` — log completion (playback continues draining buffer)
  - [x] 3.4: On `ServerMsg::Text(_)` — log as unexpected (ignoring)
  - [x] 3.5: On `ServerMsg::Error(msg)` — log warning with error text
  - [x] 3.6: On disconnect (EOF/BrokenPipe) — log and signal shutdown via AtomicBool

- [x] Task 4: Rewire main.rs for TCP + playback (AC: #1, #2)
  - [x] 4.1: Replace `RemoteTranscriber` usage with `TcpConnection`
  - [x] 4.2: Create playback channel: `crossbeam_channel::bounded(32)` for TTS audio chunks
  - [x] 4.3: Start playback stream via `playback::start_playback()`
  - [x] 4.4: Spawn `tcp_reader` thread with the TcpStream reader half + playback sender
  - [x] 4.5: VAD/audio loop sends AudioSegment via `write_client_msg` on writer half
  - [x] 4.6: Add `--server` CLI arg for IP:port (default `127.0.0.1:9500`), replacing SSH host/model args
  - [x] 4.7: Update TUI setup to configure server address instead of SSH parameters

- [x] Task 5: Integration tests (AC: #4)
  - [x] 5.1: Test TCP connection: `connect_receives_ready` — mock server sends Ready → client connects
  - [x] 5.2: Test TTS audio reception: `split_send_audio_and_read_response` — mock server sends TtsAudioChunk + TtsEnd → client receives correct samples
  - [x] 5.3: Test AudioSegment sending: same test verifies client sends AudioSegment → mock server receives correctly
  - [x] 5.4: Test `connect_rejects_non_ready` — server sends Error → client rejects
  - [x] 5.5: Test `is_disconnect_detects_eof` and `is_disconnect_ignores_other_errors`

- [x] Task 6: Remove old SSH code (AC: #1)
  - [x] 6.1: Remove `remote.rs` (RemoteTranscriber, SSH subprocess management)
  - [x] 6.2: Remove `mod remote;` from main.rs
  - [x] 6.3: Remove SSH-related CLI args from main.rs and TUI (ssh_target, remote_model_path, language)
  - [x] 6.4: Clean up unused imports; inject.rs kept with `#[allow(dead_code)]`

- [x] Task 7: Verify build passes (AC: #4)
  - [x] 7.1: Run `make check` — fmt + clippy + all tests pass
  - [x] 7.2: 61 tests pass (22 client + 26 common + 3 orchestrator + 10 server) — 5 new client tests, no regressions

## Dev Notes

### CRITICAL: Client Architecture — Before vs After

**Before (SSH model):**
```
main.rs:
  TUI setup → SSH host/model selection
  RemoteTranscriber::new(host, model, language) → spawns SSH subprocess
  1 transcriber thread: VAD audio → AudioSegment → SSH stdin → Text response → inject
  hotkey thread: evdev monitoring
```

**After (TCP daemon model):**
```
main.rs:
  TUI setup → server IP:port configuration
  TcpConnection::connect("192.168.1.10:9500") → TCP socket + Ready handshake
  1 tcp_reader thread: reads ServerMsg from TCP → routes TtsAudioChunk to playback
  1 audio/VAD thread: captures → VAD → AudioSegment → TCP writer
  1 cpal playback: output stream fed by crossbeam channel
  hotkey thread: evdev monitoring (unchanged)
```

### CRITICAL: Threading Model — 3 Workers + Main

```
Main Thread:
  TUI setup → TCP connect → wait Ready → start playback → spawn workers → main loop

tcp_reader thread:
  OWNS: BufReader<TcpStream> (reader half)
  LOOP: read ServerMsg
    → TtsAudioChunk: send to playback_tx channel
    → TtsEnd: log "TTS complete"
    → Error: log warning
    → EOF: signal shutdown

audio_sender thread (existing, modified):
  OWNS: BufWriter<TcpStream> (writer half), audio capture stream
  LOOP: VAD detects speech end → AudioSegment → write_client_msg to TCP

playback (cpal callback, not a thread):
  OWNS: crossbeam Receiver<Vec<i16>>
  CALLBACK: pop samples from channel, write to output buffer
  If channel empty: fill with silence (no stalling)

hotkey thread (unchanged):
  OWNS: evdev device
  LOOP: monitor keypress → send events
```

### CRITICAL: Stream Cloning for Split Read/Write

Same pattern as story 2-3 server session.rs:
```rust
let tcp_stream = TcpStream::connect(addr)?;
let tcp_for_read = tcp_stream.try_clone()?;  // reader → tcp_reader thread
let tcp_for_write = tcp_stream;               // writer → audio_sender thread
```

### CRITICAL: TcpConnection Design

Replace `remote.rs` RemoteTranscriber (SSH subprocess) with `connection.rs`:

```rust
pub struct TcpConnection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl TcpConnection {
    pub fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .context("connecting to server")?;
        let reader = BufReader::new(stream.try_clone()?);
        let writer = BufWriter::new(stream);

        // Wait for Ready from server
        let msg = read_server_msg(&mut reader)?;
        match msg {
            ServerMsg::Ready => info!("[client] Server ready"),
            other => anyhow::bail!("Expected Ready, got {other:?}"),
        }

        Ok(Self { reader, writer })
    }

    pub fn send_audio(&mut self, samples: &[i16]) -> Result<()> {
        write_client_msg(&mut self.writer, &ClientMsg::AudioSegment(samples.to_vec()))
    }

    pub fn read_server_msg(&mut self) -> Result<ServerMsg> {
        read_server_msg(&mut self.reader)
    }

    /// Split into reader and writer for separate thread ownership
    pub fn into_split(self) -> (BufReader<TcpStream>, BufWriter<TcpStream>) {
        (self.reader, self.writer)
    }
}
```

### Audio Playback — cpal Output Stream

Architecture decision G3 specifies: "playback starts immediately on first TtsAudioChunk, no pre-buffering."

```rust
pub fn start_playback(
    device: &cpal::Device,
    audio_rx: crossbeam_channel::Receiver<Vec<i16>>,
) -> Result<cpal::Stream> {
    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16000),
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
            let mut offset = 0;
            while offset < data.len() {
                match audio_rx.try_recv() {
                    Ok(chunk) => {
                        let n = chunk.len().min(data.len() - offset);
                        data[offset..offset + n].copy_from_slice(&chunk[..n]);
                        offset += n;
                        // If chunk has leftover samples, they're lost (acceptable for MVP)
                    }
                    Err(_) => {
                        // No data available — fill remainder with silence
                        data[offset..].fill(0);
                        break;
                    }
                }
            }
        },
        |err| warn!("[client] Playback error: {err}"),
        None,
    )?;

    Ok(stream)
}
```

**Important:** The cpal callback receives `&mut [i16]` directly if the device supports i16. If the device uses f32 natively, use `build_output_stream` with f32 and convert. Check `device.supported_output_configs()` for format selection.

### Sample Rate Mismatch Handling

Server sends 16kHz mono i16. Client's audio device may not support 16kHz. Strategy:

1. Query `device.default_output_config()` for native format
2. If native == 16kHz → direct feed (no resampling)
3. If native != 16kHz (likely 48kHz) → resample 16k→48k using rubato (same pattern as audio.rs `create_resampler()`)
4. Log actual device sample rate at info level

### Playback Channel Sizing

```rust
let (playback_tx, playback_rx) = crossbeam_channel::bounded::<Vec<i16>>(32);
```

- 32 chunks max = 32 × 4000 samples = 8 seconds of audio buffer
- Bounded prevents unbounded memory growth if playback is slow
- If buffer full, sender blocks (backpressure from playback)

### EOF/Disconnect Detection Pattern

Reuse `is_disconnect()` from story 2-3:
```rust
fn is_disconnect(e: &anyhow::Error) -> bool {
    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
        matches!(
            io_err.kind(),
            ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
        )
    } else {
        false
    }
}
```

### Current main.rs Flow to Modify

Current flow (from source analysis):
```rust
fn main() {
    // 1. TUI setup (device, hotkey, SSH host, model)
    // 2. RemoteTranscriber::new(host, model, language) → SSH subprocess
    // 3. Spawn transcriber thread: VAD audio → SSH → Text → inject
    // 4. Hotkey monitoring
    // 5. Ctrl+C shutdown
}
```

New flow:
```rust
fn main() {
    // 1. TUI setup (device, hotkey, server address)
    // 2. TcpConnection::connect(addr) → TCP + Ready handshake
    // 3. Start playback stream (cpal output + channel)
    // 4. Split connection: reader → tcp_reader thread, writer → audio thread
    // 5. Spawn tcp_reader thread: reads ServerMsg → routes to playback
    // 6. Spawn audio/VAD thread: captures → VAD → AudioSegment → TCP writer
    // 7. Hotkey monitoring (unchanged)
    // 8. Ctrl+C shutdown
}
```

### Existing Code to Preserve Unchanged

- `audio.rs` — capture functions (start_capture, create_resampler)
- `vad.rs` — VoiceActivityDetector (speech detection)
- `hotkey.rs` — evdev monitoring
- `inject.rs` — text injection (used for transcription display)
- `tui.rs` — TUI setup wizard (needs small modifications for server address)

### TUI Modifications (Minimal)

Current TUI collects: audio device, hotkey, SSH host, model, language.

For TCP mode:
- **Replace** SSH host/model/language with: server address (IP:port)
- **Keep**: audio device selection, hotkey selection
- **Add**: server address input (default: `127.0.0.1:9500`)

If TUI complexity is a concern, a CLI `--server` arg is the minimum viable approach. TUI can be updated in a follow-up.

### No New External Dependencies

All needed crates are already in client/Cargo.toml:
- `cpal` 0.17.3 — audio output (already used for capture)
- `crossbeam-channel` 0.5.15 — playback channel
- `space_lt_common` — protocol, logging
- `anyhow` — error handling
- `std::net::TcpStream` — TCP connection (stdlib)

### What text injection (inject.rs) Does

Currently, `inject.rs` takes transcribed text from the SSH server and "injects" it via `ydotool` (simulates keyboard typing). In the TCP model, this is still useful for displaying what the user said. The transcription result now comes indirectly (the server transcribes and sends to the orchestrator, not back to the client).

For story 2-4, text injection of transcribed text is NOT part of the flow — the client sends audio, receives TTS. The client no longer receives the transcription text. This functionality may be revisited in story 2-5 or dropped. **Do NOT break inject.rs** but the main loop no longer needs to call it for transcription display.

### Previous Story Intelligence (from Stories 2-2, 2-3)

- Workspace: 4 crates, `make check` passes (56 tests: 17 client + 26 common + 3 orchestrator + 10 server)
- Package naming: `space_lt_*` (underscore)
- Makefile: always use `make check` not raw cargo commands
- Clippy: `-D warnings` — all warnings are errors
- Error handling: `anyhow::Result` + `.context()` — NOT `map_err` except for eyre::Report boundaries
- Logging: `[client]` prefix, `debug!()` for verbose, `info!()` for normal
- Test convention: inline `#[cfg(test)]` modules, `match`-based assertions
- Protocol functions already flush internally — no need for explicit `flush()` after write_*_msg calls
- EOF detection: use `is_disconnect()` with `downcast_ref::<std::io::Error>()`, NOT string matching
- Stream cloning: `TcpStream::try_clone()` for split read/write — no Mutex needed
- Shutdown: use `shutdown(Shutdown::Both)` to unblock threads stuck on blocking reads
- User preference: use `cargo add` for new dependencies
- CLAUDE.md: never mention Claude in commits, always use Makefile targets

### References

- [Source: architecture.md#Communication Architecture] — TCP for client, daemon model
- [Source: architecture.md#Gap Resolutions G3] — Audio playback starts on first TtsAudioChunk (no pre-buffer)
- [Source: architecture.md#Gap Resolutions G5] — Audio pipeline recovery patterns
- [Source: architecture.md#Audio & Protocol Conventions] — TtsAudioChunk(0x83), TtsEnd(0x84), 16kHz mono i16
- [Source: architecture.md#Concurrency & Resource Patterns] — OS threads + crossbeam-channel
- [Source: architecture.md#Project Structure] — `connection.rs`, `playback.rs` file placement
- [Source: epics.md#Story 2.4] — Acceptance criteria
- [Source: 2-3-server-dual-listeners-and-message-routing.md] — TCP stream cloning, is_disconnect(), shutdown patterns
- [Source: 2-2-tts-engine-integration.md] — TTS output format (16kHz mono i16, chunked 4000 samples)
- [Source: client/src/remote.rs] — Current SSH-based RemoteTranscriber to replace
- [Source: client/src/audio.rs] — cpal capture patterns, resampler infrastructure
- [Source: client/src/main.rs] — Current threading model, TUI setup, channel patterns
- [Source: common/src/protocol.rs] — ServerMsg types, read_server_msg, write_client_msg functions

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

- `make check` output: 63 tests pass (20 client + 30 common + 3 orchestrator + 10 server)

### Completion Notes List

- `send_audio()` method removed from TcpConnection — after `into_split()` the writer half is used directly via `write_client_msg()`, making a pre-split send method unnecessary
- Playback resampling (16kHz→device native rate) done in tcp_reader thread, not in cpal callback — avoids Send bound issues with rubato resampler closure
- cpal 0.17.3 uses `SampleRate` as type alias for `u32` (not a newtype struct) — StreamConfig fields take u32 directly
- Playback callback includes leftover buffer to avoid dropping partial chunks that don't fit in the output buffer
- TUI simplified: removed SSH target, model selection, and language screens; added server address input with default "127.0.0.1:9500" (pressing Enter on empty input uses default)
- `inject.rs` preserved with `#[allow(dead_code)]` — module compiles and tests pass but is not used in main flow
- `check_input_group()` simplified: removed /dev/uinput check (only needed for dotool/inject), kept input group check (needed for evdev hotkey)
- **Review fixes**: connect timeout (10s), TCP_NODELAY, `is_disconnect()` moved to common crate, playback leftover buffer capped at 1s, 4 `is_disconnect` tests added to common

### File List

- client/src/connection.rs (new) — TcpConnection with connect timeout + TCP_NODELAY, re-exports is_disconnect from common, 3 tests
- client/src/playback.rs (new) — start_playback() with cpal output stream, bounded leftover buffer
- client/src/main.rs (modified) — TCP + playback architecture, tcp_reader_loop()
- client/src/tui.rs (modified) — simplified for server address + hotkey only
- client/src/remote.rs (deleted) — SSH RemoteTranscriber removed
- common/src/protocol.rs (modified) — added is_disconnect() + 4 tests
- server/src/session.rs (modified) — uses is_disconnect from common instead of local copy
