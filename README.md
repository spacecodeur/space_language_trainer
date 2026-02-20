# Space Language Training

Hands-free, voice-based English conversation practice tool. Pairs local speech processing (Whisper STT + Kokoro TTS on GPU) with Claude CLI to create an adaptive English tutor accessible from a tablet over the local network.

> **Note:** This project is largely designed and built with the assistance of AI (Claude). Planning artifacts (PRD, architecture, epics/stories) were collaboratively produced using the [BMAD Method](https://github.com/bmad-code-org/BMAD-METHOD), and implementation is AI-assisted.

## What it does

You speak English into a tablet, the system transcribes your speech, sends it to Claude CLI acting as a language tutor, converts Claude's response to speech, and plays it back. The whole loop runs hands-free via voice activity detection -- no keyboard needed during sessions.

Claude provides real-time grammar corrections, adapts to your CEFR level, handles diverse practice scenarios (free conversation, grammar drills, interview simulation, topic discussion with web search), and tracks your progression across sessions via markdown files.

**Key properties:**
- Zero additional API cost -- runs on existing Claude CLI subscription + local GPU
- Extends a proven Rust STT client/server codebase (`space_tts`)
- Session tracking via `.md` files with cross-session continuity

## Architecture

### 3-Process Design

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

- **Server** (desktop) -- loads Whisper large + Kokoro 82M on GPU, listens on TCP (client) and Unix socket (orchestrator), routes audio and text between them
- **Orchestrator** (desktop) -- runs the voice loop state machine, bridges to Claude CLI via `claude -p --continue`, manages session lifecycle
- **Client** (tablet) -- captures audio, detects speech via VAD, streams to server, plays back TTS audio, handles hotkey pause/resume

### Data Flow

```
1. User speaks       → client captures audio → VAD detects speech end
2. AudioSegment(0x01)→ server transcribes via Whisper
3. TranscribedText   → orchestrator receives via Unix socket
4. claude -p         → Claude CLI generates response
5. ResponseText      → server synthesizes via Kokoro TTS
6. TtsAudioChunk     → client plays audio on speakers
7. Cycle restarts at step 1
```

### Workspace Structure

```
space_language_training/
├── Cargo.toml              workspace: common, client, server, orchestrator
├── Makefile
├── common/                 protocol, models, shared types
├── client/                 audio capture + playback, VAD, hotkey, TUI
├── server/                 STT + TTS engines, TCP + Unix socket listeners
├── orchestrator/           voice loop, Claude CLI bridge, session management
└── agent/
    └── language_trainer.agent.md
```

All crates depend on `common` only. Server, client, and orchestrator communicate exclusively via the binary protocol -- no direct code dependencies between them.

### Binary Protocol

Tag-length-payload format: `[tag: u8][length: u32 LE][payload]`

| Tag | Direction | Name | Payload |
|-----|-----------|------|---------|
| `0x01` | Client → Server | AudioSegment | i16 samples LE |
| `0x02` | Client → Server | PauseRequest | empty |
| `0x03` | Client → Server | ResumeRequest | empty |
| `0x80` | Server → Client | Ready | empty |
| `0x83` | Server → Client | TtsAudioChunk | i16 samples LE |
| `0x84` | Server → Client | TtsEnd | empty |
| `0xA0` | Server → Orchestrator | TranscribedText | UTF-8 string |
| `0xA1` | Orchestrator → Server | ResponseText | UTF-8 string |
| `0xA2` | Orchestrator → Server | SessionStart | UTF-8 JSON |
| `0xA3` | Orchestrator → Server | SessionEnd | empty |

### Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| TTS engine | Kokoro 82M (Rust/ONNX) | Fits VRAM budget (2-3 Go + Whisper 3 Go on 16 Go GPU) |
| LLM integration | Claude CLI `claude -p --continue` | Zero cost, crash-isolated per turn |
| Communication | TCP (remote) + Unix socket (local) | Low-latency audio + fast local IPC |
| Concurrency | OS threads + crossbeam-channel | Consistent with existing codebase, no async runtime |
| Error handling | anyhow everywhere | Simple, sufficient for solo-dev MVP |
| Abstractions | `TtsEngine` + `LlmBackend` traits | Enable mock-based testing and future engine swaps |

## Session Tracking

```
~/language-training/
├── sessions/
│   ├── 2026-02-20T19-00_session.md    per-session synthesis
│   └── ...
├── progression.md                      chronological session summaries
├── meta.md                             CEFR level, focus areas
├── weak-points.md                      recurring error patterns
└── vocabulary.md                       cumulative vocabulary journal
```

## Requirements

- Linux/Fedora (desktop + tablet)
- NVIDIA GPU with 16 Go VRAM (tested on 4080)
- Claude CLI subscription
- Rust toolchain

## Planning Artifacts

Full project planning is available in `_bmad-output/planning-artifacts/`:
- `product-brief-*.md` -- product vision and scope
- `prd.md` -- 34 functional + 14 non-functional requirements
- `architecture.md` -- all technical decisions and patterns
- `epics.md` -- 5 epics, 17 stories with acceptance criteria
- `implementation-readiness-report-*.md` -- pre-implementation validation
