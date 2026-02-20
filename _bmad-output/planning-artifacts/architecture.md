---
stepsCompleted: [1, 2, 3, 4, 5, 6, 7, 8]
lastStep: 8
status: 'complete'
completedAt: '2026-02-20'
inputDocuments: ['prd.md', 'product-brief-space_language_training-2026-02-20.md', 'prd-validation-report.md']
workflowType: 'architecture'
project_name: 'space_language_training'
user_name: 'Matthieu'
date: '2026-02-20'
---

# Architecture Decision Document

_This document builds collaboratively through step-by-step discovery. Sections are appended as we work through each architectural decision together._

## Project Context Analysis

### Requirements Overview

**Functional Requirements:**
34 FRs across 7 capability areas. Architecturally, they decompose into 4 distinct system boundaries:
1. **Audio I/O layer** (FR1-FR7, FR31-FR34) — Rust client/server handling capture, streaming, VAD, STT, TTS, playback
2. **Orchestration layer** (FR8-FR12, FR14-FR18) — Voice loop state machine, Claude CLI subprocess management, hotkey handling, error recovery
3. **AI conversation layer** (FR25-FR30) — Entirely handled by Claude via agent definition file. No Rust code; behavioral requirements for the `language_trainer.agent.md` prompt
4. **Session tracking layer** (FR13, FR19-FR24) — Markdown file I/O handled by Claude natively. Orchestrator responsible for loading context at session start

**Non-Functional Requirements:**
14 NFRs driving architectural decisions:
- **Latency chain** (NFR1-5): Each stage has a budget within the 5-second envelope — VAD (500ms), STT, Claude CLI (up to 30s grace), TTS streaming, playback (200ms). Architecture must enable parallel/streaming where possible
- **Streaming requirement** (NFR3, NFR8): TTS must stream before full synthesis. Audio protocol must support interleaved bidirectional streams. Rules out batch-and-send approaches
- **Stability** (NFR10-14): 60-minute sessions, atomic file writes, graceful audio recovery. Demands robust error handling at every pipeline stage

**Scale & Complexity:**

- Primary domain: Real-time audio streaming + CLI orchestration (Rust)
- Complexity level: Medium
- Estimated architectural components: 6 (audio capture client, audio playback client, STT engine, TTS engine, orchestrator, Claude CLI bridge)

### Technical Constraints & Dependencies

- **Existing codebase:** `space_tts` Rust workspace (common/client/server crates) with working STT pipeline, VAD, SSH-based audio streaming, Whisper GPU acceleration
- **Hardware:** NVIDIA 4080/16Go VRAM — shared between Whisper large and TTS model. VRAM budget is a hard constraint
- **Claude CLI:** Programmatic mode (`claude -p`, `--continue`/`--resume`) is the only LLM interface. No direct API calls. Zero additional cost
- **TTS model:** Orpheus (Python-based) — requires bridging strategy for Rust integration (subprocess, FFI, or local HTTP service)
- **Platform:** Linux/Fedora only (desktop server + tablet client)
- **Network:** Local network only. No cloud dependencies beyond Claude CLI's internet access
- **Timeline:** 2-3 days total development, Phase 0 spike as go/no-go gate

### Cross-Cutting Concerns Identified

- **Latency management** — Every component in the chain (VAD → STT → Claude → TTS → playback) contributes to the 5-second budget. Architecture must enable streaming and parallelism wherever possible
- **VRAM sharing** — Whisper and TTS model compete for 16Go. Model loading order, memory management, and potential offloading strategies affect server startup and runtime
- **Error propagation** — Audio errors, Claude CLI timeouts, network drops all need graceful handling without session restart. Consistent error strategy across all layers
- **Session state** — Pause/resume must cleanly suspend all pipeline stages. Crash recovery must not corrupt tracking files (atomic writes)
- **Agent definition decoupling** — `language_trainer.agent.md` must work independently of the LLM backend (NFR9). Orchestrator passes it as system prompt; no coupling to Claude-specific features in the agent file

## Starter Template Evaluation

### Primary Technology Domain

Rust real-time audio processing + CLI orchestration — **brownfield extension** of existing `space_tts` workspace.

### Existing Foundation Assessment

No starter template required. The project extends a proven, production-grade Rust workspace with the following established decisions:

**Language & Runtime:**
- Rust (Edition 2024), Cargo workspace with 3 crates (common, client, server)
- No async runtime — OS threads with crossbeam-channel for IPC

**Audio Processing Stack:**
- cpal (0.17.3) — cross-platform audio I/O (capture)
- rubato (1.0.1) — high-quality resampling to 16 kHz mono
- webrtc-vad (0.4.0) — voice activity detection (10ms frames, 500ms silence threshold)
- audioadapter-buffers (2.0.0) — audio format conversion

**STT Engine:**
- whisper-rs (0.15.1) — Rust bindings to Whisper.cpp, optional CUDA via feature flag
- BeamSearch(beam=5), hallucination filtering, language-specific prompting

**System Integration:**
- evdev (0.13.2) — Linux hotkey monitoring
- ratatui (0.30.0) — interactive TUI setup at startup
- SSH-based client/server communication via stdin/stdout pipes

**Binary Protocol:**
- Custom tag-length-payload format: `[tag: u8][length: u32 LE][payload]`
- Extensible — tags 0x02-0x7F (client) and 0x83-0xFF (server) available
- Unit-tested round-trip serialization

### What the Existing Foundation Provides

| Decision | Status | Details |
|----------|--------|---------|
| Language | Decided | Rust |
| Audio capture | Working | cpal + rubato resampling |
| VAD | Working | webrtc-vad, 500ms silence threshold |
| STT | Working | Whisper.cpp on GPU via whisper-rs |
| Wire protocol | Working, extensible | Binary tag-length-payload |
| Hotkey | Working | evdev push-to-talk |
| TUI setup | Working | ratatui config wizard |
| Client/Server IPC | Working | SSH stdin/stdout pipes |
| Audio playback | Not present | Needs cpal output stream |
| TTS engine | Not present | Needs Orpheus/alternative integration |
| Orchestrator | Not present | New component: voice loop + Claude CLI bridge |
| Claude CLI bridge | Not present | New component: subprocess management |

### Extensions Required

1. **Protocol extension** — Add `TtsAudioChunk(0x83)`, `TtsEnd(0x84)`, `PauseRequest(0x02)`, `ResumeRequest(0x03)` message types
2. **TTS engine integration** — Orpheus (Python) bridging into Rust server (subprocess, FFI, or local HTTP)
3. **Audio playback pipeline** — cpal output stream on client for TTS audio
4. **Orchestrator** — New component managing the voice loop state machine and Claude CLI subprocess
5. **Session tracking** — Markdown file I/O delegated to Claude via agent definition

### Starter Decision

**No external starter template.** Extend the existing `space_tts` workspace directly. The proven architecture, working audio pipeline, and extensible protocol provide a stronger foundation than any generic CLI starter could offer.

## Core Architectural Decisions

### Decision Priority Analysis

**Critical Decisions (Block Implementation):**
1. TTS engine choice
2. Orchestrator placement
3. Client-server communication architecture
4. Claude CLI invocation strategy
5. Workspace structure

**Deferred Decisions (Implementation Phase):**
- TTS audio output format (dictated by Kokoro)
- Session directory structure
- Error handling patterns (step 5)

### TTS Engine

- **Decision:** Kokoro 82M via Rust native ONNX inference
- **Rationale:** Fits VRAM budget (2-3 Go + Whisper 3 Go = 5-6 Go on 16 Go GPU). Native Rust implementations available (Kokoros, Kokorox). Quality comparable to larger models at fraction of the resource cost.
- **Abstraction:** `TtsEngine` trait with streaming API from day one. Engine selection via `--tts-model` CLI argument. Enables future swap to Orpheus (via HTTP to Orpheus-FastAPI server) or other engines.

```rust
trait TtsEngine {
    fn synthesize_stream(&self, text: &str) -> Box<dyn Iterator<Item = AudioChunk>>;
}
```

- **Affects:** server crate, common crate (protocol), client crate (playback)

### LLM Backend Abstraction

- **Decision:** `LlmBackend` trait abstracting Claude CLI invocation, enabling mock-based testing
- **Rationale:** Claude CLI is an external non-controllable component. Without abstraction, the orchestrator cannot be tested without real Claude CLI calls (slow, flaky, costly). Mirror pattern of `TtsEngine`.

```rust
trait LlmBackend {
    fn query(&self, prompt: &str, system_prompt_file: &Path, continue_session: bool) -> Result<String>;
}
```

Two implementations:
- `ClaudeCliBackend` — spawns `claude -p`, captures stdout
- `MockLlmBackend` — returns predefined responses for testing

- **Affects:** orchestrator crate

### Orchestrator Architecture

- **Decision:** Separate process on desktop machine, communicating with server via Unix socket
- **Rationale:** Clean separation of responsibilities (audio processing vs conversation management). Server restart without 60s model reload. Independent debugging and testing. ~150 LOC overhead for significantly better maintainability and evolvability.
- **Connection model:** Server listens on Unix socket, orchestrator connects at startup. Server is long-lived (models loaded), orchestrator is ephemeral (restartable).
- **Testing strategy:** Each component testable in isolation — unit tests, integration tests, and manual E2E validation at every step.
- **Affects:** New `orchestrator` crate, server crate (Unix socket listener)

### Communication Architecture

- **Decision:** TCP for remote client (tablet), Unix socket for local orchestrator
- **Rationale:** TCP provides low-latency bidirectional streaming for audio between tablet and desktop. Unix socket is more performant for local IPC between server and orchestrator. Clean channel separation.
- **Protocol:** Existing tag-length-payload binary format (`[tag: u8][length: u32 LE][payload]`) reused on both channels. New message types added for TTS and control.
- **Execution model change:** Moving from SSH-launched server to daemon model. Server runs independently, listening on TCP + Unix socket. Requires launch script or systemd service.
- **Affects:** common crate (protocol extension), server crate (dual listeners), client crate (TCP client), orchestrator crate (Unix socket client)

### Claude CLI Integration

- **Decision:** One `claude -p` invocation per conversation turn, `--continue` for session continuity
- **Rationale:** Simplest and most robust approach. Each turn is an independent process — crash isolation, no long-lived process management. Fork/exec overhead (~200-500ms) is negligible vs Claude response time (~2-5s). `--continue` provides native session context preservation.
- **Invocation pattern:** `claude -p --system-prompt-file language_trainer.agent.md --continue "transcribed text"`
- **Critical prerequisite — Phase 0 spike must validate:**
  - `--continue` preserves context over 20+ sequential turns
  - `--system-prompt-file` + `--continue` work together correctly
  - stdout capture is clean (no stderr pollution)
  - Fork/exec overhead is acceptable
  - If spike fails, project architecture is reassessed
- **Affects:** orchestrator crate

### Workspace Structure

- **Decision:** Git fork of `space_tts` into `space_language_training` as independent workspace. Original `space_tts` preserved and frozen (no further changes — bug fixes go into the fork).
- **Rationale:** Freedom to modify all crates without breaking existing STT tool. Original remains available as reference.
- **Crate layout:**

```
space_language_training/
├── Cargo.toml              (workspace: common, client, server, orchestrator)
├── common/                 (protocol, models, shared types)
├── client/                 (audio capture + playback, VAD, hotkey, TUI)
├── server/                 (STT + TTS engines, TCP + Unix socket listeners)
└── orchestrator/           (voice loop, Claude CLI bridge, session management)
```

- **Affects:** All crates

### Test Strategy by Crate

| Crate | Unit Tests | Integration Tests | Manual E2E |
|-------|-----------|-------------------|------------|
| `common` | Protocol serialization round-trip, all message types | — | — |
| `server` | TtsEngine mock, TCP listener, Unix socket listener | Real STT + mock orchestrator | Launch server, send audio, verify transcription |
| `orchestrator` | LlmBackend mock, state machine, session management | Real orchestrator + mock server | Launch orchestrator, type text, verify Claude call |
| `client` | Audio playback mock, protocol encoding | Real client + mock server | Launch client, speak, hear response |
| **Full E2E** | — | — | All 3 processes, 5-minute voice conversation |

### Decision Impact Analysis

**Implementation Sequence:**
1. **Phase 0 spike** — Validate Claude CLI `--continue` over 20+ turns (blocker for entire project)
2. Fork workspace and rename (foundation)
3. Extend protocol with new message types (common)
4. Build orchestrator with Claude CLI bridge + LlmBackend trait (orchestrator) — highest risk first
5. Add TTS engine integration with TtsEngine trait (server)
6. Replace SSH with TCP listener (server)
7. Add Unix socket listener for orchestrator (server)
8. Add audio playback pipeline (client)
9. Wire everything together (E2E)

**Cross-Component Dependencies:**
- Phase 0 spike is a hard gate — no code until validated
- Protocol changes (common) must happen first — all crates depend on it
- Orchestrator (highest risk) developed before TTS (known problem) — fail fast principle
- TTS integration (server) and audio playback (client) can be developed in parallel after orchestrator works
- Orchestrator depends on server's Unix socket listener being ready for integration testing

## Implementation Patterns & Consistency Rules

### Critical Conflict Points Identified

5 areas where AI agents could make different choices, all resolved below.

### Naming Conventions

- **Rust standard enforced:** `snake_case` functions/variables, `CamelCase` types/traits/enums, `SCREAMING_SNAKE_CASE` constants
- **Protocol message variants:** `CamelCase` enum variants (consistent with existing `AudioSegment`, `Ready`, `Text`, `Error`)
- **Crate names:** `snake_case` in code, hyphenated in Cargo.toml (`space-lt-common`)
- **Log messages:** Prefixed with component name: `[server]`, `[orchestrator]`, `[client]`

### Test Organization

- **Unit tests:** Inline `#[cfg(test)]` modules in the same file as the code under test
- **Integration tests:** `tests/` directory at crate root, testing public API
- **Test philosophy:** Pragmatic coverage, not dogmatic. Test what matters — protocol serialization, state machine transitions, trait mocks. No tests for trivial code (getters, simple wrappers).
- **Every new protocol message type** must have a round-trip serialization test in `common`

### Error Handling

- **`anyhow::Result` everywhere** — consistent with existing `space_tts` codebase
- No custom error types for MVP. Refine later if debugging becomes painful.
- Error context via `anyhow::Context` (`.context("loading TTS model")`) for actionable error messages

### Logging

- **Existing custom macros** (`info!()`, `debug!()`, `warn!()`) based on `eprintln!`
- **Component prefix** in all messages: `[server] Model loaded`, `[orchestrator] Claude CLI timeout`, `[client] Connected`
- `debug!()` for verbose output (gated by `--debug` flag), `info!()` for normal operation, `warn!()` for recoverable errors

### Audio & Protocol Conventions

**Audio format:**
- STT input: 16 kHz mono i16 (imposed by Whisper, already in place)
- TTS output: server normalizes Kokoro output to 16 kHz mono i16 before sending to client
- Client always receives i16 16kHz mono — one format, no negotiation

**Protocol extension rules:**
- Client → Server tags: `0x01-0x7F` (existing: `0x01` AudioSegment)
- Server → Client tags: `0x80-0xFF` (existing: `0x80` Ready, `0x81` Text, `0x82` Error)
- Orchestrator ↔ Server tags: `0xA0-0xBF` (new dedicated namespace, Unix socket only)

**New message types:**

| Tag | Direction | Name | Payload |
|-----|-----------|------|---------|
| `0x02` | Client → Server | `PauseRequest` | empty |
| `0x03` | Client → Server | `ResumeRequest` | empty |
| `0x83` | Server → Client | `TtsAudioChunk` | i16 samples LE |
| `0x84` | Server → Client | `TtsEnd` | empty |
| `0xA0` | Server → Orchestrator | `TranscribedText` | UTF-8 string |
| `0xA1` | Orchestrator → Server | `ResponseText` | UTF-8 string |
| `0xA2` | Orchestrator → Server | `SessionStart` | UTF-8 JSON (config) |
| `0xA3` | Orchestrator → Server | `SessionEnd` | empty |

### Concurrency & Resource Patterns

- **Threading model:** OS threads + `crossbeam-channel` (no async runtime) — consistent with existing codebase
- **Graceful shutdown:** `ctrlc` handler + shutdown channel broadcast to all threads (existing pattern)
- **Atomic file writes:** Write to temp file + `fs::rename()` for session tracking files — crash cannot corrupt existing data
- **Resource cleanup:** RAII for model handles, `Drop` implementations for network connections

### Enforcement Guidelines

**All AI agents working on this codebase MUST:**
1. Run `cargo fmt` and `cargo clippy` before considering any code complete
2. Add a round-trip test for any new protocol message type
3. Use `anyhow::Context` on all fallible operations with human-readable context
4. Prefix log messages with component name
5. Follow existing code patterns in `space_tts` when extending modules

## Project Structure & Boundaries

### Complete Project Directory Structure

```
space_language_training/
├── Cargo.toml                          # workspace: common, client, server, orchestrator
├── Cargo.lock
├── Makefile                            # build, test, check, run targets
├── README.md
├── setup.sh                            # extended from space_tts (+ TTS model download)
│
├── common/
│   ├── Cargo.toml                      # deps: anyhow
│   ├── src/
│   │   ├── lib.rs                      # module exports
│   │   ├── protocol.rs                 # all message types (TCP + Unix socket)
│   │   ├── models.rs                   # model path resolution (STT + TTS)
│   │   └── log.rs                      # info!/debug!/warn! macros
│   └── tests/
│       └── protocol_integration.rs     # cross-message-type serialization scenarios
│
├── client/
│   ├── Cargo.toml                      # deps: common, cpal, rubato, webrtc-vad, evdev, ratatui, crossbeam-channel
│   ├── src/
│   │   ├── main.rs                     # entry: TUI setup → TCP connect → audio loop
│   │   ├── audio.rs                    # cpal capture + resampling (existing)
│   │   ├── playback.rs                 # cpal output stream for TTS audio (NEW)
│   │   ├── vad.rs                      # voice activity detection (existing)
│   │   ├── connection.rs               # TCP client to server (REPLACES remote.rs/SSH)
│   │   ├── hotkey.rs                   # evdev hotkey monitoring (existing)
│   │   └── tui.rs                      # ratatui setup wizard (EXTENDED: server IP/port)
│   └── tests/
│       └── playback_integration.rs     # audio output with mock server
│
├── server/
│   ├── Cargo.toml                      # deps: common, whisper-rs, kokoro crate, anyhow
│   ├── src/
│   │   ├── main.rs                     # entry: load models → start listeners
│   │   ├── listener.rs                 # TCP (client) + Unix socket (orchestrator) (NEW)
│   │   ├── transcribe.rs              # Whisper STT integration (existing)
│   │   ├── tts.rs                      # TtsEngine trait + KokoroTts impl (NEW)
│   │   └── session.rs                  # routes messages between client ↔ orchestrator (NEW)
│   └── tests/
│       ├── tts_integration.rs          # TtsEngine with mock audio verification
│       └── listener_integration.rs     # TCP + Unix socket accept/message exchange
│
├── orchestrator/
│   ├── Cargo.toml                      # deps: common, anyhow
│   ├── src/
│   │   ├── main.rs                     # entry: connect to server → voice loop
│   │   ├── claude.rs                   # LlmBackend trait + ClaudeCliBackend (NEW)
│   │   ├── voice_loop.rs              # state machine: idle → listening → processing → speaking (NEW)
│   │   └── session.rs                  # context loading, session lifecycle (NEW)
│   └── tests/
│       ├── claude_integration.rs       # LlmBackend with MockLlmBackend
│       └── voice_loop_integration.rs   # state machine transitions with mocks
│
└── agent/
    └── language_trainer.agent.md       # Claude agent definition (NEW)
```

### Session Directory Structure (runtime)

```
~/language-training/
├── language_trainer.agent.md           # agent definition (copied or symlinked)
├── sessions/
│   ├── 2026-02-20T19-00_session.md    # per-session synthesis (FR19)
│   ├── 2026-02-21T19-15_session.md
│   └── ...
├── progression.md                      # chronological session summaries (FR20)
├── meta.md                             # CEFR level, NZ countdown, focus areas (FR21)
├── weak-points.md                      # recurring error patterns (FR22)
└── vocabulary.md                       # cumulative vocabulary journal (FR23)
```

### Architectural Boundaries

**Process Boundaries (3 binaries):**

```
┌─────────────────────────────────────────────────────────┐
│ Desktop machine                                          │
│                                                          │
│  ┌─────────────┐  Unix socket   ┌──────────────────┐   │
│  │ server      │◄──────────────►│ orchestrator      │   │
│  │             │  (0xA0-0xBF)   │                   │   │
│  │ - Whisper   │                │ - voice loop      │   │
│  │ - Kokoro    │                │ - Claude CLI      │   │
│  │ - listeners │                │ - session mgmt    │   │
│  └──────┬──────┘                └───────────────────┘   │
│         │ TCP                                            │
│         │ (0x01-0x84)                                    │
└─────────┼────────────────────────────────────────────────┘
          │ local network
┌─────────┼──────────┐
│ Tablet  │          │
│  ┌──────┴──────┐   │
│  │ client      │   │
│  │ - capture   │   │
│  │ - playback  │   │
│  │ - VAD       │   │
│  │ - hotkey    │   │
│  └─────────────┘   │
└────────────────────┘
```

**Crate Dependency Boundaries:**

```
common ←── client
common ←── server
common ←── orchestrator
(server and orchestrator have NO dependency on each other — communication via protocol only)
```

### FR → Structure Mapping

| FR Category | Crate(s) | Key Files |
|-------------|----------|-----------|
| Voice Input (FR1-4) | client + server | `audio.rs`, `vad.rs`, `transcribe.rs` |
| Voice Output (FR5-7) | server + client | `tts.rs`, `playback.rs` |
| Conversation Mgmt (FR8-13) | orchestrator | `claude.rs`, `session.rs` |
| Session Lifecycle (FR14-18) | orchestrator + client | `voice_loop.rs`, `hotkey.rs` |
| Progress Tracking (FR19-24) | orchestrator + Claude | `session.rs`, `language_trainer.agent.md` |
| Language Coaching (FR25-30) | Claude agent only | `language_trainer.agent.md` |
| Infrastructure (FR31-34) | server + client + common | `listener.rs`, `connection.rs`, `protocol.rs` |

### Data Flow

```
1. User speaks → client/audio.rs captures → client/vad.rs detects speech end
2. client/connection.rs sends AudioSegment(0x01) → server/listener.rs receives
3. server/transcribe.rs → Whisper STT → TranscribedText(0xA0) → orchestrator via Unix socket
4. orchestrator/claude.rs spawns: claude -p --system-prompt-file agent.md --continue "text"
5. Claude response → orchestrator sends ResponseText(0xA1) → server via Unix socket
6. server/tts.rs → Kokoro synthesize_stream() → TtsAudioChunk(0x83) → client via TCP
7. client/playback.rs plays audio → TtsEnd(0x84) → cycle restarts at step 1
```

### Development Workflow

**Makefile targets:**

```makefile
build:             cargo build --workspace
check:             cargo fmt --check && cargo clippy --workspace && cargo test --workspace
test:              cargo test --workspace
test-common:       cargo test -p space-lt-common
test-server:       cargo test -p space-lt-server
test-orchestrator: cargo test -p space-lt-orchestrator
test-client:       cargo test -p space-lt-client
run-server:        cargo run -p space-lt-server -- --stt-model large --tts-model kokoro
run-orchestrator:  cargo run -p space-lt-orchestrator -- --agent agent/language_trainer.agent.md
run-client:        cargo run -p space-lt-client -- --server 192.168.1.10:9500
```

## Architecture Validation Results

### Coherence Validation

**Decision Compatibility:** PASS — All technology choices (Kokoro ONNX, whisper-rs, cpal, crossbeam, TCP/Unix socket) are compatible. No framework conflicts. VRAM budget (5-6 Go on 16 Go) confirmed viable.

**Pattern Consistency:** PASS — 4-crate workspace with common dependency graph supports all decisions. Trait abstractions (TtsEngine, LlmBackend) align with testing strategy. Protocol tag namespaces are collision-free across transports.

**Structure Alignment:** PASS — Every FR maps to a specific crate/file. Boundaries between processes are clean (protocol-only communication). No crate depends on another except through common.

**Contradictions:** None found. PRD ambiguities (Orpheus vs alternative, orchestrator placement) are resolved by explicit architecture decisions with documented rationale.

### Requirements Coverage

**Functional Requirements:** 33/34 covered. FR18 (retry on timeout) had a gap — resolved below (G1).

**Non-Functional Requirements:** 8/14 fully covered, 4 partial, 2 gaps — resolved below (G1-G6).

### Gap Resolutions

**G1 — Claude CLI Timeout & Retry Pattern (CRITICAL — FR18, NFR7, NFR12):**

`ClaudeCliBackend::query()` implements:
1. Spawn `claude -p` subprocess
2. Wait with 30-second timeout (kill process if exceeded — NFR7)
3. On failure: retry up to 3 times with 5-second intervals (FR18, NFR12)
4. On total failure: return predefined error string "I'm sorry, I'm having trouble connecting right now. Please wait a moment."
5. Orchestrator sends error string as `ResponseText(0xA1)` → server synthesizes via TTS → user hears the error

**G2 — Pause/Resume Propagation (IMPORTANT — NFR11):**

Server acts as the gate — orchestrator has no pause awareness:
1. Client sends `PauseRequest(0x02)` to server
2. Server stops forwarding `TranscribedText` to orchestrator (drops incoming audio segments)
3. Server stops sending `TtsAudioChunk` to client (if TTS mid-stream: flush remaining chunks, send `TtsEnd`)
4. On `ResumeRequest(0x03)`: server resumes forwarding in both directions
5. Orchestrator continues running unchanged — pause is transparent to it

**G3 — Audio Playback Buffering (IMPORTANT — NFR5):**

`playback.rs` starts cpal output stream immediately on first `TtsAudioChunk`. No pre-buffering. Accept potential initial underrun (inaudible click) rather than adding latency. Ring buffer feeds cpal callback. Trade-off documented: latency over smoothness.

**G4 — Model Loading Order (IMPORTANT — NFR4):**

Sequential loading in `server/main.rs`:
1. Load Whisper model first (critical path, larger model, ~3 Go VRAM)
2. Load Kokoro model second (~2-3 Go VRAM)
3. Fail-fast: if either model fails, server exits with error
4. Log VRAM usage after each model load (debug level)

**G5 — Audio Pipeline Error Recovery (IMPORTANT — NFR14):**

Recovery patterns by error type:
- cpal device error → attempt stream restart up to 3 times, then report via TUI and exit
- TCP connection drop (client) → client attempts reconnection with exponential backoff (1s, 2s, 4s), max 3 attempts
- Buffer underrun → log warning, continue (self-healing, no user impact)
- Resampling error → skip audio segment, log warning

**G6 — Session Start Handshake (IMPORTANT):**

Startup sequence:
1. Server starts, listens on TCP + Unix socket. Accepts client TCP connections but buffers audio (does not process).
2. Orchestrator connects via Unix socket, sends `SessionStart(0xA2)` with config JSON (agent file path, session directory)
3. Server acknowledges with `Ready(0x80)` on Unix socket
4. Server begins processing client audio (STT → forward to orchestrator)
5. First transcribed text triggers orchestrator's voice loop

### Deferred Items (Post-MVP)

- G7: InterruptTts message type (PauseRequest implicitly interrupts for MVP)
- G8: PRD `--system-prompt` vs architecture `--system-prompt-file` inconsistency (documentation fix)
- G9: Error-to-voice mechanism detail (covered by G1 fix above for Claude CLI; other error types use TUI)
- G10: Makefile creation (first implementation step)
- G11: Barge-in handling (MVP: client mutes capture while TTS plays — simplest approach)
- G12: Server `session.rs` routing table (AudioSegment→STT→orchestrator, ResponseText→TTS→client, PauseResume→server state)

### Architecture Completeness Checklist

**Requirements Analysis**
- [x] Project context thoroughly analyzed
- [x] Scale and complexity assessed (Medium)
- [x] Technical constraints identified (VRAM, Claude CLI, Linux-only)
- [x] Cross-cutting concerns mapped (latency, VRAM, errors, session state, agent decoupling)

**Architectural Decisions**
- [x] TTS engine: Kokoro 82M via Rust ONNX, TtsEngine trait
- [x] LLM backend: Claude CLI via subprocess, LlmBackend trait
- [x] Orchestrator: separate process, Unix socket to server
- [x] Communication: TCP (client) + Unix socket (orchestrator)
- [x] Claude CLI: one `claude -p` per turn, `--continue` for context
- [x] Workspace: git fork of space_tts, 4 crates

**Implementation Patterns**
- [x] Naming conventions (Rust standard + protocol CamelCase + log prefixes)
- [x] Test organization (inline unit + tests/ integration, pragmatic coverage)
- [x] Error handling (anyhow everywhere)
- [x] Logging (custom macros, component prefix)
- [x] Audio format (16kHz mono i16 normalized)
- [x] Protocol extension rules (tag namespaces, round-trip tests)
- [x] Concurrency (OS threads + crossbeam, RAII, atomic file writes)

**Project Structure**
- [x] Complete directory structure (4 crates + agent/)
- [x] Component boundaries (3 processes, protocol-only IPC)
- [x] FR → structure mapping (all 34 FRs mapped)
- [x] Data flow (7-step pipeline documented)
- [x] Session directory structure (5 tracking files)

### Architecture Readiness Assessment

**Overall Status:** READY FOR IMPLEMENTATION

**Confidence Level:** High — all critical gaps resolved, all FRs and NFRs covered, coherence validated.

**Key Strengths:**
- Clean 3-process architecture with protocol-only boundaries
- Trait abstractions enable testing without hardware/network
- Existing proven codebase (space_tts) provides solid foundation
- Phase 0 spike as hard gate before full development investment
- Pragmatic patterns (anyhow, OS threads, custom macros) aligned with solo dev + 2-3 day timeline

**Areas for Future Enhancement:**
- Async runtime if latency optimization needed
- `tracing` crate if debugging across processes becomes complex
- Custom error types if anyhow context strings prove insufficient
- Barge-in / echo cancellation for more natural conversation flow

### Implementation Handoff

**AI Agent Guidelines:**
- Follow all architectural decisions exactly as documented
- Use implementation patterns consistently across all crates
- Respect crate boundaries — no direct dependencies between server/client/orchestrator
- Refer to this document for all architectural questions
- When in doubt, follow existing `space_tts` code patterns

**First Implementation Priority:**
1. Phase 0 spike — validate Claude CLI `--continue` over 20+ turns (no Rust code)
2. Fork workspace, create Makefile, set up 4-crate structure
3. Extend protocol (common crate) with all new message types + tests
