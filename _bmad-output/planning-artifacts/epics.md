---
stepsCompleted: [1, 2, 3, 4]
status: complete
inputDocuments: ['prd.md', 'architecture.md']
---

# space_language_training - Epic Breakdown

## Overview

This document provides the complete epic and story breakdown for space_language_training, decomposing the requirements from the PRD and Architecture into implementable stories.

## Requirements Inventory

### Functional Requirements

**Voice Input (Speech-to-Text)**
- FR1: User can speak into the tablet microphone and have speech captured and streamed to the server
- FR2: System can detect speech start and end automatically via Voice Activity Detection (no manual trigger needed)
- FR3: System can transcribe English speech to text using Whisper in English-only mode
- FR4: System can process speech segments incrementally (transcribe as user speaks, not after full recording)

**Voice Output (Text-to-Speech)**
- FR5: System can convert text responses to English speech with quality sufficient to sustain 30-60 minute listening sessions
- FR6: System can stream generated audio from server to client for playback on the tablet
- FR7: User can hear Claude's responses through the tablet's audio output

**Conversation Management**
- FR8: System can invoke Claude CLI programmatically in non-interactive mode
- FR9: System can maintain conversation context across multiple turns within a session
- FR10: System can load a standalone agent definition file and pass it as system prompt to Claude CLI
- FR11: System can provide Claude with the contents of tracking files at session start for context awareness
- FR12: Claude can perform web searches during conversation without requiring user approval
- FR13: Claude can read and write files in the session directory (tracking `.md` files)

**Session Lifecycle**
- FR14: User can start a session by launching the client and connecting to the server
- FR15: System can run a continuous voice conversation loop (listen -> transcribe -> Claude -> TTS -> play -> listen) without keyboard interaction
- FR16: User can pause and resume STT+TTS via a configurable hotkey
- FR17: User can end a session by returning to the keyboard and quitting
- FR18: System can retry Claude CLI requests up to 3 times on network timeout, reporting failure via audio prompt if all retries fail

**Progress Tracking**
- FR19: System can generate a timestamped per-session synthesis file at session end (topics, errors, corrections, vocabulary, assessment)
- FR20: System can update a general progression document with a chronological summary of each session
- FR21: System can maintain a meta tracking document with overall CEFR level, NZ departure countdown, and suggested focus areas
- FR22: System can maintain a recurring weak points tracker listing persistent error patterns across sessions
- FR23: System can maintain a vocabulary journal accumulating new words and expressions with usage context
- FR24: System can load previous tracking files at session start to provide continuity across sessions

**Language Coaching**
- FR25: Claude can provide real-time grammar and vocabulary corrections during conversation (default mode)
- FR26: User can vocally request deferred feedback mode (corrections saved for end of session or mini-session)
- FR27: Claude can adapt conversation vocabulary and grammar complexity based on the CEFR level recorded in the meta tracking document
- FR28: Claude can handle the following scenario types requested vocally: free conversation, grammar drills, interview simulation, topic discussion with web search, and level assessment — without formal mode switching
- FR29: Claude can conduct an initial level assessment when no previous tracking files exist
- FR30: Claude can suggest session focus areas based on remaining time before NZ trip and identified weak points

**Infrastructure**
- FR31: Server can load and initialize both STT and TTS models at startup
- FR32: Client can connect to the server over the local network via IP and port
- FR33: Client can configure hotkey preference at startup
- FR34: Server and client can exchange bidirectional audio and control messages

### NonFunctional Requirements

**Performance**
- NFR1: End-to-end response latency (speech end detected → audio response starts playing) must be under 5 seconds for 90% of turns
- NFR2: VAD silence detection must trigger within 500ms of actual speech end to avoid cutting off the user or waiting too long
- NFR3: TTS audio generation must begin streaming to client before full response is synthesized (streaming TTS, not batch)
- NFR4: STT and TTS model loading at server startup must complete within 60 seconds
- NFR5: Audio playback on client must start within 200ms of receiving the first TTS audio chunk

**Integration**
- NFR6: Claude CLI invocation must support session continuity via `--continue` or `--resume` with no context loss between turns within a session
- NFR7: Orchestrator must handle Claude CLI response times up to 30 seconds without treating it as a failure
- NFR8: Audio protocol between client and server must support interleaved STT (client→server) and TTS (server→client) streams without collision or data loss
- NFR9: Agent definition file (`language_trainer.agent.md`) must be loadable by the orchestrator without dependency on a specific LLM backend

**Reliability**
- NFR10: System must sustain a 60-minute continuous voice session without crashes, memory leaks, or audio pipeline degradation
- NFR11: Hotkey pause/resume must respond within 200ms and cleanly suspend/resume both STT and TTS pipelines
- NFR12: If Claude CLI becomes temporarily unreachable (network interruption), orchestrator must retry up to 3 times with 5-second intervals before reporting failure to user
- NFR13: Session tracking files must be written atomically — a crash during file generation must not corrupt existing tracking data
- NFR14: Audio pipeline must recover gracefully from transient errors (dropped packets, buffer underruns) without requiring session restart

### Additional Requirements

**From Architecture — Critical Decisions:**
- Phase 0 spike is a hard gate: validate Claude CLI `--continue` over 20+ turns before any Rust code
- Git fork of `space_tts` into `space_language_training` as independent workspace
- 4-crate workspace: common, client, server, orchestrator
- TTS engine: Kokoro 82M via Rust native ONNX inference, behind `TtsEngine` streaming trait
- LLM backend: `LlmBackend` trait abstracting Claude CLI, with `MockLlmBackend` for testing
- Orchestrator: separate process communicating with server via Unix socket
- Communication: TCP for remote client (tablet), Unix socket for local orchestrator
- Claude CLI: one `claude -p` per turn with `--continue` for session continuity
- Server operates as daemon (TCP + Unix socket listeners), replacing SSH execution model

**From Architecture — Protocol Extension:**
- 8 new message types: PauseRequest(0x02), ResumeRequest(0x03), TtsAudioChunk(0x83), TtsEnd(0x84), TranscribedText(0xA0), ResponseText(0xA1), SessionStart(0xA2), SessionEnd(0xA3)
- Protocol tag namespaces: client→server 0x01-0x7F, server→client 0x80-0xFF, orchestrator↔server 0xA0-0xBF

**From Architecture — Implementation Patterns:**
- OS threads + crossbeam-channel (no async runtime)
- anyhow::Result everywhere with Context for error handling
- Atomic file writes (temp + rename) for session tracking
- Sequential model loading: Whisper first, then Kokoro
- Audio format: 16kHz mono i16 normalized everywhere
- Pragmatic test coverage: unit tests inline, integration tests in tests/

**From Architecture — Gap Resolutions:**
- G1: ClaudeCliBackend timeout (30s) + 3 retries (5s intervals) + error-to-voice fallback
- G2: Pause/resume propagation handled by server (transparent to orchestrator)
- G3: Audio playback starts immediately on first TtsAudioChunk (no pre-buffering)
- G4: Model loading order: Whisper first, Kokoro second, fail-fast
- G5: Audio pipeline recovery patterns per error type (device error, TCP drop, buffer underrun)
- G6: Session start handshake: orchestrator sends SessionStart → server acks with Ready → processing begins

**From Architecture — No Starter Template:**
- Brownfield extension of existing space_tts (working STT, VAD, audio capture, binary protocol, hotkey, TUI)
- No UX design document (CLI-based project, no GUI)

### FR Coverage Map

FR1: Epic 2 — Audio capture and streaming (existing from space_tts, adapted to TCP)
FR2: Epic 2 — VAD speech detection (existing from space_tts)
FR3: Epic 2 — Whisper STT transcription (existing from space_tts)
FR4: Epic 2 — Incremental speech processing (existing from space_tts)
FR5: Epic 2 — TTS text-to-speech conversion (Kokoro engine)
FR6: Epic 2 — TTS audio streaming server→client
FR7: Epic 2 — Audio playback on tablet
FR8: Epic 1 (spike) + Epic 2 — Claude CLI programmatic invocation
FR9: Epic 1 (spike) + Epic 2 — Conversation context via --continue
FR10: Epic 2 — Agent definition file loading as system prompt
FR11: Epic 5 — Load tracking files into Claude context at session start
FR12: Epic 4 — Web search integration in conversations
FR13: Epic 5 — Claude reads/writes session tracking files
FR14: Epic 2 — Session start via client connection
FR15: Epic 2 — Continuous voice conversation loop
FR16: Epic 3 — Hotkey pause/resume
FR17: Epic 3 — Session end (return to keyboard, quit)
FR18: Epic 3 — Claude CLI retry on timeout (3 attempts, audio error prompt)
FR19: Epic 5 — Per-session synthesis file generation
FR20: Epic 5 — General progression document update
FR21: Epic 5 — Meta tracking document maintenance
FR22: Epic 5 — Recurring weak points tracker
FR23: Epic 5 — Vocabulary journal maintenance
FR24: Epic 5 — Load previous tracking files for session continuity
FR25: Epic 4 — Real-time grammar and vocabulary corrections
FR26: Epic 4 — Deferred feedback mode on vocal request
FR27: Epic 4 — CEFR-adaptive conversation complexity
FR28: Epic 4 — Scenario handling (free conversation, grammar, interview, topic+web, assessment)
FR29: Epic 4 — Initial level assessment (no prior tracking files)
FR30: Epic 4 — Session focus suggestions based on NZ countdown + weak points
FR31: Epic 2 — STT + TTS model loading at startup
FR32: Epic 2 — Client TCP connection to server
FR33: Epic 3 — Hotkey configuration at startup (TUI)
FR34: Epic 1 (protocol) + Epic 2 — Bidirectional audio and control messages

## Epic List

### Epic 1: Project Foundation & Feasibility Validation
The user can confirm that the project is technically feasible and the development workspace is ready for implementation.
- Fork `space_tts` into `space_language_training`, set up 4-crate workspace with Makefile
- Phase 0 spike: validate Claude CLI `--continue` over 20+ sequential turns (hard gate — project reassessed if spike fails)
- Extend binary protocol with all 8 new message types + round-trip serialization tests
**FRs covered:** FR8 (partial), FR9 (partial), FR34 (partial)

### Epic 2: End-to-End Voice Conversation
The user can have a complete, hands-free voice conversation with Claude from the tablet over the local network.
- Server: Kokoro TTS engine (TtsEngine trait), TCP listener (client), Unix socket listener (orchestrator), sequential model loading (Whisper → Kokoro)
- Client: TCP connection (replaces SSH), TTS audio playback pipeline (cpal output)
- Orchestrator: voice loop state machine, Claude CLI bridge (LlmBackend trait), basic agent prompt
- E2E: speak → VAD → STT → orchestrator → Claude CLI → TTS → audio playback → loop
**FRs covered:** FR1, FR2, FR3, FR4, FR5, FR6, FR7, FR8, FR9, FR10, FR14, FR15, FR31, FR32, FR34

### Epic 3: Session Control & Robustness
The user can handle real-life interruptions gracefully and benefit from stable, resilient sessions.
- Hotkey pause/resume with server-side propagation (transparent to orchestrator)
- Hotkey configuration via TUI at startup
- Claude CLI timeout (30s) + retry (3 attempts, 5s intervals) + audio error prompt
- Audio pipeline error recovery (device error, TCP drop, buffer underrun)
- Clean session end handling
**FRs covered:** FR16, FR17, FR18, FR33

### Epic 4: Intelligent Language Coaching
Claude acts as an adaptive English tutor with real-time corrections, diverse scenarios, and CEFR-based level assessment.
- Complete `language_trainer.agent.md`: coaching persona, CEFR methodology, feedback rules
- Real-time corrections (default) + deferred feedback on vocal request
- Initial level assessment when no tracking files exist
- Scenario handling: free conversation, grammar drills, interview simulation, topic discussion with web search, level assessment
- Focus area suggestions based on NZ departure countdown and identified weak points
- Web search integration for topic-based discussions
**FRs covered:** FR12, FR25, FR26, FR27, FR28, FR29, FR30

### Epic 5: Cross-Session Progress Tracking
The user can track English progression across sessions with automated synthesis, context continuity, and persistent learning insights.
- Per-session synthesis file generation (timestamped)
- 5 tracking files: session synthesis, progression, meta (CEFR + NZ countdown), weak points, vocabulary journal
- Context loading at session start (tracking files → Claude context)
- Session lifecycle handshake (SessionStart/SessionEnd messages)
- Atomic file writes (temp + rename) for crash safety
**FRs covered:** FR11, FR13, FR19, FR20, FR21, FR22, FR23, FR24

### Epic Dependencies

```
Epic 1 (Foundation)
  └──► Epic 2 (Voice Conversation)
         ├──► Epic 3 (Session Control)    [independent]
         ├──► Epic 4 (Language Coaching)   [independent]
         └──► Epic 5 (Progress Tracking)  [independent]
```

Epics 3, 4, and 5 are independent of each other. They all depend on Epic 2 but can be implemented in any order.

## Epic 1: Project Foundation & Feasibility Validation

The user can confirm that the project is technically feasible and the development workspace is ready for implementation.

### Story 1.1: Fork Workspace and Set Up Project Structure

As a **developer**,
I want to fork `space_tts` into an independent `space_language_training` workspace with 4 crates,
So that I have a clean development foundation without breaking the original STT project.

**Acceptance Criteria:**

**Given** the existing `space_tts` repository
**When** the developer forks it into `space_language_training`
**Then** the workspace contains 4 crates: `common`, `client`, `server`, `orchestrator`
**And** `orchestrator` crate is created with minimal `main.rs` (hello world)
**And** `Cargo.toml` workspace members include all 4 crates
**And** `Makefile` provides targets: `build`, `check`, `test`, `test-common`, `test-server`, `test-orchestrator`, `test-client`
**And** `make build` succeeds without errors
**And** `make check` passes (fmt + clippy + test)

### Story 1.2: Validate Claude CLI Session Continuity (Phase 0 Spike)

As a **developer**,
I want to validate that Claude CLI `--continue` preserves conversation context over 20+ sequential turns,
So that I can confirm the core technical assumption before investing in Rust development.

**Acceptance Criteria:**

**Given** Claude CLI is installed and functional
**When** the developer runs a scripted test sending 20+ sequential prompts with `claude -p --continue`
**Then** Claude's responses demonstrate context awareness of the full conversation history
**And** `--system-prompt-file` combined with `--continue` works correctly (system prompt is respected across all turns)
**And** stdout capture is clean (no stderr pollution in captured output)
**And** fork/exec overhead per turn is documented (target: <500ms)
**And** a go/no-go decision is documented based on results
**And** if the spike fails, the project is reassessed (hard gate)

### Story 1.3: Extend Binary Protocol with New Message Types

As a **developer**,
I want to extend the binary protocol with all 8 new message types for TTS, pause/resume, and orchestrator communication,
So that the protocol foundation is ready for all future epics.

**Acceptance Criteria:**

**Given** the existing protocol in `common/src/protocol.rs` with `AudioSegment(0x01)`, `Ready(0x80)`, `Text(0x81)`, `Error(0x82)`
**When** the developer adds the 8 new message types
**Then** the following messages are implemented: `PauseRequest(0x02)`, `ResumeRequest(0x03)`, `TtsAudioChunk(0x83)`, `TtsEnd(0x84)`, `TranscribedText(0xA0)`, `ResponseText(0xA1)`, `SessionStart(0xA2)`, `SessionEnd(0xA3)`
**And** tag namespaces are respected: client→server `0x01-0x7F`, server→client `0x80-0xFF`, orchestrator↔server `0xA0-0xBF`
**And** each new message type has a round-trip serialization unit test (encode → decode → verify equality)
**And** `SessionStart(0xA2)` payload is UTF-8 JSON (config structure)
**And** `make test-common` passes with all new tests

## Epic 2: End-to-End Voice Conversation

The user can have a complete, hands-free voice conversation with Claude from the tablet over the local network.

### Story 2.1: Orchestrator Claude CLI Bridge

As a **developer**,
I want the orchestrator to communicate with Claude CLI programmatically via a `LlmBackend` trait,
So that I can validate the highest-risk component first and enable mock-based testing.

**Acceptance Criteria:**

**Given** the orchestrator crate with `LlmBackend` trait defined
**When** the developer implements `ClaudeCliBackend` and `MockLlmBackend`
**Then** `ClaudeCliBackend` spawns `claude -p --system-prompt-file <path> --continue "text"` and captures stdout
**And** `MockLlmBackend` returns predefined responses for testing
**And** `LlmBackend::query()` accepts prompt, system_prompt_file path, and continue_session boolean
**And** a basic `language_trainer.agent.md` file exists with minimal English tutor persona
**And** integration test with `MockLlmBackend` verifies the query interface works correctly
**And** manual E2E test: run orchestrator standalone, type text in terminal, receive Claude response on stdout

### Story 2.2: TTS Engine Integration (Kokoro)

As a **developer**,
I want the server to synthesize English speech from text using Kokoro via a streaming `TtsEngine` trait,
So that Claude's responses can be converted to natural-sounding audio.

**Acceptance Criteria:**

**Given** the server crate with `TtsEngine` trait defined (`synthesize_stream(&self, text: &str) -> Box<dyn Iterator<Item = AudioChunk>>`)
**When** the developer implements `KokoroTts`
**Then** `KokoroTts` loads the Kokoro 82M model via ONNX runtime
**And** `synthesize_stream()` returns audio chunks incrementally (streaming, not batch)
**And** output audio format is 16kHz mono i16 (normalized from Kokoro native format)
**And** integration test verifies synthesized audio chunks are non-empty and correctly formatted
**And** manual E2E test: feed a text string, write output to WAV file, verify audio is intelligible English speech
**And** VRAM usage after model load is logged at debug level

### Story 2.3: Server Dual Listeners and Message Routing

As a **developer**,
I want the server to accept TCP connections from the client and Unix socket connections from the orchestrator, routing messages between them,
So that all three processes can communicate via the extended binary protocol.

**Acceptance Criteria:**

**Given** the server with Whisper STT and Kokoro TTS loaded
**When** the server starts as a daemon
**Then** it listens on a configurable TCP port (default 9500) for client connections
**And** it listens on a Unix socket for orchestrator connections
**And** models are loaded sequentially: Whisper first, then Kokoro (fail-fast if either fails)
**And** model loading completes within 60 seconds (NFR4)
**And** `AudioSegment(0x01)` from client is transcribed via Whisper and forwarded as `TranscribedText(0xA0)` to orchestrator
**And** `ResponseText(0xA1)` from orchestrator is synthesized via TTS and streamed as `TtsAudioChunk(0x83)` + `TtsEnd(0x84)` to client
**And** integration test: mock client sends AudioSegment, mock orchestrator receives TranscribedText; mock orchestrator sends ResponseText, mock client receives TtsAudioChunk
**And** manual E2E test: launch server, connect with netcat/test client, verify message flow

### Story 2.4: Client TCP Connection and Audio Playback

As a **developer**,
I want the client to connect to the server via TCP and play received TTS audio through the tablet speakers,
So that the user can hear Claude's spoken responses.

**Acceptance Criteria:**

**Given** the existing client with audio capture, VAD, and resampling
**When** the developer replaces SSH communication with TCP and adds audio playback
**Then** client connects to server via TCP at specified IP:port (replaces `remote.rs`/SSH)
**And** existing audio capture and VAD continue to work over TCP (AudioSegment sent via TCP)
**And** `TtsAudioChunk(0x83)` messages are received and fed to a cpal output stream for playback
**And** playback starts within 200ms of first TtsAudioChunk received (NFR5)
**And** ring buffer feeds cpal audio callback to handle timing differences
**And** `TtsEnd(0x84)` signals end of current TTS response
**And** integration test: mock server sends TtsAudioChunk sequence, client receives and decodes correctly
**And** manual E2E test: client connected to running server, hear TTS audio on speakers

### Story 2.5: Voice Loop and End-to-End Integration

As a **user**,
I want to have a complete hands-free voice conversation with Claude from my tablet,
So that I can practice English without touching the keyboard.

**Acceptance Criteria:**

**Given** server, orchestrator, and client are all running and connected
**When** the user speaks into the tablet microphone
**Then** the full conversation loop executes: speech → VAD detection → STT transcription → orchestrator → Claude CLI → TTS synthesis → audio playback → ready for next turn
**And** orchestrator voice loop state machine transitions correctly: idle → listening → processing → speaking → idle
**And** session start handshake works: orchestrator sends `SessionStart(0xA2)`, server responds with `Ready(0x80)`, processing begins
**And** end-to-end latency is under 5 seconds for 90% of turns (NFR1)
**And** conversation context is maintained across turns via `--continue` (NFR6)
**And** the system sustains a 5-minute continuous conversation without crashes
**And** manual E2E test: launch all 3 processes, conduct a multi-turn voice conversation

## Epic 3: Session Control & Robustness

The user can handle real-life interruptions gracefully and benefit from stable, resilient sessions.

### Story 3.1: Hotkey Pause/Resume

As a **user**,
I want to pause and resume the conversation via a hotkey on my tablet,
So that I can handle real-life interruptions without losing my session.

**Acceptance Criteria:**

**Given** an active voice conversation session
**When** the user presses the configured pause hotkey
**Then** client sends `PauseRequest(0x02)` to server
**And** server stops forwarding `TranscribedText` to orchestrator (drops incoming audio segments)
**And** server stops sending `TtsAudioChunk` to client (if TTS mid-stream: flush remaining chunks, send `TtsEnd`)
**And** pause takes effect within 200ms (NFR11)
**And** orchestrator continues running unchanged (pause is transparent to it)

**Given** a paused session
**When** the user presses the resume hotkey
**Then** client sends `ResumeRequest(0x03)` to server
**And** server resumes forwarding in both directions
**And** conversation continues from where it was paused
**And** manual E2E test: mid-conversation, press pause, verify silence, press resume, verify conversation continues

### Story 3.2: Hotkey Configuration and Session End

As a **user**,
I want to configure my preferred hotkey at startup and cleanly end a session,
So that I can choose a key that works with my tablet and exit gracefully.

**Acceptance Criteria:**

**Given** the client is launching
**When** the TUI setup wizard runs at startup
**Then** hotkey selection is available (extended from existing `space_tts` TUI)
**And** server IP and port configuration is included in TUI

**Given** an active session
**When** the user returns to the keyboard and quits (Ctrl+C or quit command)
**Then** client sends a clean disconnect to server
**And** orchestrator detects session end and performs cleanup
**And** all processes shut down gracefully without hanging threads or leaked resources
**And** manual E2E test: configure hotkey in TUI, start session, quit cleanly, verify no zombie processes

### Story 3.3: Claude CLI Retry and Audio Error Recovery

As a **user**,
I want the system to handle network timeouts and audio glitches automatically,
So that my session continues smoothly despite transient errors.

**Acceptance Criteria:**

**Given** an active conversation session
**When** Claude CLI does not respond within 30 seconds (NFR7)
**Then** orchestrator kills the subprocess and retries (up to 3 attempts, 5-second intervals)
**And** if all retries fail, orchestrator sends a predefined error string as `ResponseText(0xA1)`
**And** server synthesizes the error message via TTS and user hears it as audio prompt
**And** conversation can continue on next user turn

**Given** a transient audio pipeline error
**When** a cpal device error occurs on client
**Then** client attempts stream restart up to 3 times before reporting error via TUI and exiting
**When** a TCP connection drop occurs
**Then** client attempts reconnection with exponential backoff (1s, 2s, 4s), max 3 attempts
**When** a buffer underrun occurs
**Then** system logs warning and continues (self-healing, no user impact)

**And** integration test with `MockLlmBackend`: simulate timeout, verify retry behavior and error message
**And** manual E2E test: disconnect internet mid-conversation, verify retry and audio error prompt

## Epic 4: Intelligent Language Coaching

Claude acts as an adaptive English tutor with real-time corrections, diverse scenarios, and CEFR-based level assessment.

### Story 4.1: Core Language Coaching Persona and Real-Time Feedback

As a **user**,
I want Claude to act as a patient, encouraging English tutor who corrects my grammar and vocabulary in real time,
So that I improve through natural conversation with immediate feedback.

**Acceptance Criteria:**

**Given** the `language_trainer.agent.md` file loaded as system prompt
**When** the user makes a grammar error (e.g., "I have went to the store")
**Then** Claude provides a natural, inline correction (e.g., "Quick note — the correct form is 'I have gone'. Try again?")
**And** corrections are concise and don't break conversation flow
**And** Claude adapts vocabulary and grammar complexity based on the CEFR level recorded in the meta tracking document (FR27)
**And** the agent persona is encouraging and patient, suitable for sustained 30-60 min sessions
**And** the agent definition file is LLM-backend-agnostic (no Claude-specific features referenced) (NFR9)
**And** manual E2E test: conduct a 10-minute conversation with deliberate errors, verify corrections are natural and accurate

### Story 4.2: Deferred Feedback and Scenario Handling

As a **user**,
I want to switch between real-time and deferred feedback modes, and request different practice scenarios vocally,
So that I can tailor each session to my learning needs.

**Acceptance Criteria:**

**Given** an active conversation with real-time feedback (default)
**When** the user says "let's switch to deferred feedback" or similar vocal request
**Then** Claude acknowledges and stops inline corrections, saving them for session summary
**And** the user can switch back to real-time feedback vocally

**Given** an active conversation
**When** the user requests a scenario vocally (e.g., "let's do an interview simulation", "can we practice grammar?", "let's discuss a topic")
**Then** Claude seamlessly transitions to the requested scenario without formal mode switching
**And** the following scenario types are supported: free conversation, grammar drills, interview simulation, topic discussion with web search, level assessment (FR28)
**And** web search is used when the user requests topic-based discussion (FR12)
**And** manual E2E test: switch feedback modes vocally, request 3 different scenarios, verify smooth transitions

### Story 4.3: Initial Level Assessment and Focus Suggestions

As a **user**,
I want Claude to assess my English level on first use and suggest focus areas based on my weak points and NZ trip timeline,
So that my practice is targeted and efficient.

**Acceptance Criteria:**

**Given** no previous tracking files exist (first session)
**When** the session starts
**Then** Claude detects the absence of tracking files and initiates a conversational level assessment (FR29)
**And** assessment covers: speaking fluency, grammar accuracy, vocabulary range, listening comprehension
**And** after assessment, Claude estimates CEFR level and communicates it

**Given** tracking files exist with weak points and meta document (subsequent sessions)
**When** the session starts
**Then** Claude suggests focus areas based on remaining time before NZ trip (May 2026) and identified weak points (FR30)
**And** suggestions are actionable (e.g., "You have 3 months left. Based on your recurring issues with present perfect, let's focus on that today")

**And** manual E2E test: first session without tracking files — verify assessment happens; subsequent session with mock tracking files — verify focus suggestions

## Epic 5: Cross-Session Progress Tracking

The user can track English progression across sessions with automated synthesis, context continuity, and persistent learning insights.

### Story 5.1: Session Context Loading

As a **user**,
I want each session to start with awareness of my previous progress,
So that Claude picks up where we left off and doesn't repeat the same ground.

**Acceptance Criteria:**

**Given** tracking files exist in `~/language-training/` (meta.md, weak-points.md, recent session syntheses)
**When** the orchestrator starts a new session
**Then** it reads the meta tracking document, weak points tracker, and the 3 most recent session syntheses
**And** these are included in the Claude CLI invocation context (appended to the prompt or via file references)
**And** Claude demonstrates awareness of previous sessions in its greeting (e.g., references past topics, known weak points)
**And** if no tracking files exist (first session), the orchestrator proceeds without context loading (triggers assessment in Epic 4)

**Given** a `SessionStart(0xA2)` message from the orchestrator
**When** server receives it
**Then** server acknowledges with `Ready(0x80)` and begins processing client audio
**And** the session directory path is communicated via the SessionStart JSON payload

**And** integration test: mock tracking files, verify orchestrator reads and includes them in Claude context
**And** manual E2E test: after 2+ sessions, start a new session, verify Claude references previous progress

### Story 5.2: Session Synthesis and Tracking File Generation

As a **user**,
I want a detailed synthesis of each session automatically generated at session end,
So that I can review what was covered and track my improvement over time.

**Acceptance Criteria:**

**Given** an active session that is ending (user quits or says "let's wrap up")
**When** the orchestrator triggers session end
**Then** Claude generates a timestamped per-session synthesis file (e.g., `2026-02-20T19-00_session.md`) containing: topics covered, errors made, corrections given, vocabulary learned, session assessment (FR19)
**And** Claude updates the general progression document with a chronological summary of this session (FR20)
**And** Claude updates the vocabulary journal with new words and expressions from this session, with usage context (FR23)
**And** all files are written to the `~/language-training/` session directory
**And** file writes are atomic: write to temp file then `fs::rename()` — crash cannot corrupt existing data (NFR13)
**And** orchestrator sends `SessionEnd(0xA3)` to server after files are generated

**And** manual E2E test: complete a 5-minute session, verify synthesis file is generated with correct content, progression document is updated

### Story 5.3: Meta Tracking, Weak Points, and Vocabulary Maintenance

As a **user**,
I want persistent tracking of my CEFR level, recurring error patterns, and vocabulary growth across all sessions,
So that I can see long-term progress and focus on areas that need improvement.

**Acceptance Criteria:**

**Given** a session has just ended
**When** Claude generates end-of-session tracking updates
**Then** the meta tracking document (`meta.md`) is updated with: current CEFR level estimate, NZ departure countdown (May 2026), suggested focus areas for next session, milestones achieved (FR21)
**And** the recurring weak points tracker (`weak-points.md`) is updated: new error patterns added, resolved patterns marked, recurring errors highlighted with frequency (FR22)
**And** all updates are atomic (temp + rename) (NFR13)

**Given** the tracking system has data from 5+ sessions
**When** the user reviews `weak-points.md`
**Then** it shows a clear picture of persistent vs resolved error patterns
**When** the user reviews `meta.md`
**Then** it shows CEFR progression over time and remaining NZ countdown
**When** the user reviews `vocabulary.md`
**Then** it shows cumulative vocabulary growth with session-by-session entries and usage context

**And** manual E2E test: after 5+ sessions, review all tracking files, verify they accurately reflect session history
