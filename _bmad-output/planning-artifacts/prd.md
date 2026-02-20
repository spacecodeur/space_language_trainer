---
stepsCompleted: ['step-01-init', 'step-02-discovery', 'step-02b-vision', 'step-02c-executive-summary', 'step-03-success', 'step-04-journeys', 'step-05-domain', 'step-06-innovation', 'step-07-project-type', 'step-08-scoping', 'step-09-functional', 'step-10-nonfunctional', 'step-11-polish', 'step-12-complete']
status: complete
inputDocuments: ['product-brief-space_language_training-2026-02-20.md']
workflowType: 'prd'
documentCounts:
  briefs: 1
  research: 0
  brainstorming: 0
  projectDocs: 0
  projectContext: 0
classification:
  projectType: cli_tool
  domain: edtech
  complexity: medium
  projectContext: brownfield
---

# Product Requirements Document - space_language_training

**Author:** Matthieu
**Date:** 2026-02-20

## Executive Summary

Space Language Training is a hands-free, voice-based English conversation practice tool built as an extension of the existing `space_tts` Rust project. It targets a single user — an A2/B1 French-speaking developer preparing for a trip to New Zealand in May 2026 — who needs to rapidly improve spoken English fluency during daily treadmill sessions (30-60 min). The system pairs local speech processing (Whisper STT on GPU, Orpheus TTS) with Claude CLI's conversational intelligence to create an adaptive English tutor accessible from a tablet over the local network, at zero additional API cost beyond the existing Claude CLI subscription.

The core value proposition is a natural, fluid vocal dialogue experience. The system must feel close enough to a real conversation that daily 30-60 minute sessions are sustainable and engaging. Adaptive scenarios (free conversation, grammar drills, interview simulation, topic-based discussion with web research) are driven entirely by Claude's contextual intelligence — no formal scenario engine required. Session tracking via markdown files provides cross-session progression awareness.

### What Makes This Special

- **Voice quality as the primary value driver** — STT accuracy and TTS naturalness determine whether the tool gets used daily or abandoned. Orpheus-grade TTS and Whisper large model set the quality floor.
- **Zero marginal cost** — Runs entirely on existing Claude CLI subscription and local hardware (NVIDIA 4080/16Go). No additional API fees, no cloud dependencies beyond Claude.
- **Proven foundation** — Extends a battle-tested Rust STT client/server architecture with VAD, SSH-based audio streaming, and Whisper GPU acceleration already implemented and working.
- **Claude as the brain** — Unlike local LLM alternatives, leverages Claude's advanced language teaching, error correction, and adaptive conversational capabilities. Web search integration enables real-world topic discussions.
- **Hands-free by design** — VAD-based turn detection eliminates keyboard interaction during sessions. Hotkey pause/resume handles real-life interruptions.

## Project Classification

- **Type:** CLI tool with client/server audio backend
- **Domain:** EdTech (language learning)
- **Complexity:** Medium — no regulatory constraints, but requires quality integration of STT, TTS, Claude CLI orchestration, and session management
- **Context:** Brownfield — extends existing `space_tts` Rust workspace (3 crates: common, client, server) with proven STT pipeline

## Success Criteria

### User Success

- **Daily engagement sustained:** User completes 30-60 min sessions at least 5 times per week without wanting to quit early due to tool friction
- **Conversational flow:** Exchanges feel natural enough to maintain conversation without frustration — response latency under 5 seconds end-to-end (speech end → audio response starts)
- **Measurable English progression:** Reduction in recurring grammar errors visible in weak points tracker over 4+ weeks. Vocabulary journal shows steady growth. CEFR self-assessment reaches B1 solid within 1 month, B2 target by May 2026
- **Scenario versatility:** User can vocally request any scenario (free conversation, grammar drills, interview simulation, topic discussion) and Claude adapts seamlessly without formal mode switching

### Business Success

N/A — Personal preparation tool with a fixed deadline (NZ trip, May 2026). No revenue, growth, or market metrics. Success = the tool gets used daily and English improves measurably.

### Technical Success

- **STT reliability:** Whisper large model in English-only mode transcribes user speech accurately enough for Claude to detect grammar/vocabulary errors (target: >90% word accuracy for A2/B1 speaker)
- **TTS naturalness:** Orpheus or equivalent produces speech natural enough to serve as a pronunciation reference and sustain 30-60 min listening without fatigue
- **End-to-end latency:** < 5 seconds from end of user speech to start of audio response (VAD silence detection + Whisper transcription + Claude CLI round-trip + TTS generation + audio streaming)
- **Session stability:** No crashes or audio pipeline failures during 60-minute sessions
- **Claude CLI integration:** Programmatic invocation maintains conversation context across turns within a session via `--continue`/`--resume`

### Measurable Outcomes

| Outcome | Target | Measurement |
|---------|--------|-------------|
| MVP functional | End-to-end voice session works | 5 consecutive sessions without blocking bug |
| Session duration | 30-60 min sustained | Session synthesis timestamps |
| Usage frequency | 5+ sessions/week | Session file count per week |
| Grammar error trend | Decreasing | Weak points tracker over 4 weeks |
| Vocabulary growth | Steady accumulation | Vocabulary journal entries per session |
| CEFR level | B1 solid → B2 | Periodic evaluation sessions |
| Latency | < 5 seconds | Measured during sessions |

## Product Scope

### MVP - Minimum Viable Product

**Voice Conversation Loop (extends `space_tts`)**
- TTS engine added to server (Orpheus or equivalent high-quality model)
- Bidirectional audio streaming: tablet (capture/playback) ↔ desktop (STT/TTS)
- Orchestrator loop: VAD listen → Whisper STT → Claude CLI → TTS → audio playback → listen
- Hotkey pause/resume for STT+TTS

**Claude CLI Integration**
- Programmatic Claude CLI invocation (`claude -p`, `--continue`/`--resume`) for zero additional API cost
- Agent `language_trainer` with English coaching persona, CEFR-aware, real-time feedback by default, deferred on request
- Web search enabled without approval prompts
- Adaptive scenario handling — no formal scenario engine

**Session Tracking System (5 `.md` files)**
- Per-session synthesis (timestamped)
- General progression document (chronological session summaries)
- Meta tracking document (CEFR level, NZ countdown, suggested focus areas)
- Recurring weak points tracker (error patterns across sessions)
- Vocabulary journal (new words/expressions with usage context)

**Session Lifecycle**
- Start: orchestrator loads meta + recent syntheses + weak points into Claude context
- Active: hands-free voice conversation
- End: Claude generates/updates all 5 tracking files

### Growth Features (Post-MVP)

- Multiple TTS voices and accents (NZ, British, American, Australian)
- Spaced repetition integration for vocabulary journal
- Conversation history replay (re-listen to past sessions)
- Structured CEFR-aligned curriculum with formal assessments

### Vision (Future)

- Pronunciation coaching via phoneme-level analysis
- Progress dashboard (web-based visualization)
- Multi-language support beyond English
- Multi-user support

## User Journeys

### Journey 1: Happy Path — "The Daily Practice"

**Matthieu**, développeur, rentre du travail à 19h. Il enfile ses baskets, monte sur le tapis roulant dans le salon, et démarre à 5 km/h.

Il ouvre un terminal sur sa tablette posée sur le support du tapis, tape `claude` puis `/space_language_trainer`, et pose le clavier sur le côté. Une voix naturelle anglaise l'accueille : *"Good evening Matthieu! I've reviewed your last session — you made great progress on conditional tenses. Would you like to continue working on those, or try something different tonight?"*

Matthieu répond vocalement : *"I'd like to talk about something I read today — there's a new technology called OpenClaw, can we discuss it?"* Claude fait une recherche web, puis entame une conversation sur le sujet en anglais. Pendant 40 minutes, ils échangent naturellement. Claude corrige en temps réel : *"Quick note — you said 'I have went', but the correct form is 'I have gone'. It's an irregular past participle. Try again?"* Matthieu répète la forme correcte.

Vers la fin de la session, Matthieu dit *"Let's wrap up."* Claude fait un bilan oral : points travaillés, erreurs récurrentes, nouveaux mots utilisés. Il éteint le tapis, reprend le clavier, quitte Claude CLI. Cinq fichiers `.md` sont déjà à jour — la synthèse de session, la progression globale, le meta tracking, les weak points, et le vocabulaire.

**Capabilities revealed:** orchestrator loop, VAD turn detection, Claude CLI integration, web search, real-time feedback, session synthesis generation, multi-file tracking update.

---

### Journey 2: Edge Cases — "The Interrupted Session"

Matthieu est en pleine session depuis 20 minutes, discutant de la préparation de son voyage en NZ. Sa compagne entre dans le salon et lui pose une question. Il appuie sur le **hotkey pause** — le STT et TTS se coupent instantanément. Il répond en français, échange quelques mots. Il réappuie sur le hotkey, le système reprend. Claude enchaîne là où ils en étaient : *"So, you were telling me about the hiking trails you want to visit..."*

Plus tard dans la même session, Matthieu dit quelque chose que Whisper transcrit mal — *"I want to visit the Milford Track"* est transcrit *"I want to visit the milf or track"*. Claude, comprenant le contexte, ne relève pas l'erreur de transcription et répond naturellement sur le Milford Track.

À un autre moment, la connexion internet coupe pendant 30 secondes. L'orchestrateur détecte le timeout de Claude CLI, attend la reconnexion, et rejoue la dernière requête. Matthieu entend un bref silence puis la conversation reprend.

**Capabilities revealed:** hotkey pause/resume, audio pipeline suspend/resume, STT error tolerance (Claude contextual understanding), network resilience, session state preservation during interruptions.

---

### Journey 3: First Launch — "Getting Started"

Matthieu vient de compiler le projet étendu `space_language_training`. Sur sa machine fixe, il lance le serveur : `space_lt_server --model large --tts orpheus`. Le serveur charge Whisper large et le modèle Orpheus, fait un warmup GPU, et affiche `Ready — listening on port 9500`.

Sur sa tablette, il lance le client : `space_lt_client --server 192.168.1.10:9500`. Le client se connecte, teste le micro, confirme la connexion audio bidirectionnelle. Il ouvre ensuite Claude CLI avec l'agent language_trainer.

Le meta tracking document n'existe pas encore — l'agent le détecte et entame une session d'évaluation initiale : *"Welcome! This is our first session. I'd like to assess your current English level so I can adapt our future sessions. Let's start with a simple conversation — tell me about yourself and what you do for a living."*

Après 15 minutes de conversation évaluative, Claude génère les 5 fichiers de tracking initiaux : le meta document (niveau estimé A2/B1, date NZ : mai 2026, 3 mois restants), la première synthèse de session, le weak points initial (premières observations), et le vocabulaire journal. Matthieu lit les fichiers et voit un portrait fidèle de son niveau actuel.

**Capabilities revealed:** server startup with model loading, client connection setup, initial level assessment, first-time file generation, meta document initialization with NZ countdown.

---

### Journey Requirements Summary

| Capability | J1 Happy Path | J2 Edge Cases | J3 First Launch |
|-----------|:---:|:---:|:---:|
| Orchestrator voice loop | x | x | x |
| VAD turn detection | x | x | x |
| Claude CLI programmatic invocation | x | x | x |
| Web search integration | x | | |
| Real-time feedback | x | | |
| Deferred feedback | | | x |
| Session synthesis generation | x | x | x |
| Multi-file tracking update | x | x | x |
| Hotkey pause/resume | | x | |
| STT error tolerance | | x | |
| Network resilience | | x | |
| Server/client startup & connection | | | x |
| Initial level assessment | | | x |
| First-time file generation | | | x |

## Innovation & Novel Patterns

### Detected Innovation Areas

**Voice-driven Claude CLI agent** — Transformation of a text-based tool (Claude CLI) into a hands-free conversational voice interface via a local STT/TTS bridge. This combination does not exist in the current ecosystem: no open-source project connects Claude CLI to a voice loop for language learning or any other use case.

**Key innovation components:**
- Orchestrator chaining VAD → Whisper STT → Claude CLI (programmatic mode) → TTS → audio playback in an autonomous loop
- Exploitation of `claude -p` / `--continue` mode to maintain conversational context at zero additional API cost
- Specialized `language_trainer` agent within the Claude CLI agent ecosystem

### Market Context & Competitive Landscape

Confirmed by market research: existing voice-based language learning tools (Companion, Discute, RealtimeVoiceChat) all require separate API subscriptions (OpenAI, Groq) and none integrate with Claude CLI. No project combines local STT/TTS with Claude CLI's programmatic mode for any use case.

Validation is handled via Phase 0 technical spike (see Project Scoping). Risk mitigation strategy and go/no-go criteria are detailed in the Risk Mitigation Strategy section.

## CLI Tool Specific Requirements

### Project-Type Overview

Hybrid CLI tool: a Rust client/server binary pair for audio processing, combined with Claude CLI invoked programmatically for conversational AI. The user interacts via voice (no keyboard during sessions). CLI arguments configure the infrastructure; the conversational experience is driven by a standalone agent definition file loaded by the orchestrator.

### Command Structure

**Server binary: `space_lt_server`**

| Argument | Required | Description |
|----------|----------|-------------|
| `--stt-model` | Yes | Whisper model to load (tiny/base/small/medium/large) |
| `--tts-model` | Yes | TTS model to load (orpheus/kokoro/piper) |
| `--port` | No | Listening port (default: 9500) |

**Client binary: `space_lt_client`**

| Argument | Required | Description |
|----------|----------|-------------|
| `--server-ip` | Yes | Server IP address |
| `--server-port` | No | Server port (default: 9500) |

Hotkey configuration via interactive TUI at startup (consistent with existing `space_tts` approach).

### Configuration Schema

- **No persistent config file for MVP** — TUI-based setup at each launch (consistent with `space_tts`)
- **Agent definition** — `language_trainer.agent.md` standalone file defining persona, coaching methodology, CEFR framework, feedback rules, and session tracking format. Loaded by orchestrator and passed as system prompt to the LLM backend. Not coupled to any specific LLM tool.
- **Session directory** — Dedicated directory (e.g., `~/language-training/`) containing agent definition + all tracking `.md` files. Orchestrator operates from this directory.

### Technical Architecture Considerations

**Orchestrator role:**
- Runs on the desktop machine (same as server)
- Manages the voice loop: receives transcribed text from server → pipes to `claude -p` → receives response → sends to TTS
- Loads `language_trainer.agent.md` and passes it as system prompt via `--system-prompt` flag (or equivalent for other LLM backends)
- Handles session lifecycle (start/end), context loading (`--continue`/`--resume`), and hotkey events from client
- Could be integrated into the server binary or be a separate process

**Claude CLI integration:**
- Invoked via `claude -p` (print mode, non-interactive)
- Session continuity via `--continue` or `--resume <session-id>`
- System prompt loaded from `language_trainer.agent.md` by orchestrator, passed via `--system-prompt` flag
- File read/write for tracking `.md` files handled by Claude natively

**Audio protocol:**
- Extends existing `space_tts` binary protocol over SSH/TCP
- Adds TTS audio streaming (server → client) to existing STT audio streaming (client → server)
- New message types needed: `TtsAudio` (server → client), `PauseResume` (client → server)

### Implementation Considerations

- **Extend existing workspace** — Add TTS crate dependency, extend protocol with new message types, add orchestrator logic
- **Preserve existing STT pipeline** — Don't break current `space_tts` functionality while extending
- **Separate concerns** — Audio processing (Rust) vs conversational AI (Claude CLI) cleanly separated by the orchestrator boundary. Agent definition decoupled from LLM backend.

## Project Scoping & Phased Development

### MVP Strategy & Philosophy

**MVP Approach:** Problem-solving MVP with technical spike gate. Validate the core voice loop first, then build the full experience only if the spike confirms feasibility.

**Resource Requirements:** Solo developer (Matthieu), Rust-experienced, 2-3 days total development time. Leverages existing `space_tts` codebase.

**Go/No-Go Gate:** If the technical spike (Phase 0) reveals unacceptable latency, Claude CLI context issues, or TTS integration blockers, the project is reassessed before investing further.

### Phase 0: Technical Spike (Day 1 — morning)

Minimal proof-of-concept to validate the critical path:
- Whisper STT transcribes speech → text piped to `claude -p --system-prompt "You are an English tutor"` → response piped to a basic TTS (even `espeak` or Piper) → audio playback
- Validate: end-to-end latency < 5s, Claude CLI context preservation with `--continue`, audio quality acceptable
- No client/server split, no tracking files, no agent definition — just the raw loop on the desktop
- **Pass:** proceed to Phase 1. **Fail:** reassess architecture.

### Phase 1: MVP (Days 1-3)

**Core User Journeys Supported:** J1 (Happy Path), J3 (First Launch)

**Must-Have Capabilities:**
- TTS engine integrated into server (Orpheus or validated alternative from spike)
- Bidirectional audio streaming (extend `space_tts` protocol with `TtsAudio` and `PauseResume` messages)
- Orchestrator voice loop: VAD → STT → Claude CLI → TTS → playback → loop
- `language_trainer.agent.md` standalone agent definition
- Hotkey pause/resume
- Session tracking: all 5 `.md` files (session synthesis, progression, meta tracking, weak points, vocabulary journal)
- Session lifecycle: load context at start, generate/update files at end

**Acceptable shortcuts for 2-3 day timeline:**
- Network resilience (J2) — basic timeout/retry, no sophisticated reconnection
- Initial level assessment (J3) — Claude handles this via agent prompt, no special code needed
- TUI setup — reuse `space_tts` TUI patterns, minimal additions

### Phase 2: Hardening (Post-MVP, if used daily)

- Network resilience improvements (auto-reconnect, retry queue)
- STT error tolerance refinement (Whisper model tuning)
- Agent prompt iteration based on real session experience
- Session tracking format refinement based on actual usage

### Phase 3: Enhancements (Future)

- Multiple TTS voices and accents
- Spaced repetition for vocabulary
- Conversation history replay
- Structured CEFR curriculum

### Phase 4: Vision (Long-term)

- Pronunciation coaching (phoneme-level)
- Progress dashboard
- Multi-language support
- Generalization: voice-CLI bridge decoupled from language training use case

### Risk Mitigation Strategy

**Technical Risks:**
- **Claude CLI programmatic mode** — Primary risk. Mitigated by Phase 0 spike before any other development. No fallback identified; project reassessed if spike fails.
- **TTS model integration in Rust** — Orpheus is Python-based. May need to run as a subprocess or find Rust bindings. Spike will validate approach.
- **VRAM budget** — Whisper large + Orpheus on 16Go VRAM. Should fit but needs spike validation.

**Resource Risks:**
- **2-3 day timeline** — Tight but feasible given existing codebase. If scope slips, cut network resilience and TUI polish first. Core voice loop + tracking files are non-negotiable.

**Market Risks:** N/A — personal project.

## Functional Requirements

### Voice Input (Speech-to-Text)

- **FR1:** User can speak into the tablet microphone and have speech captured and streamed to the server
- **FR2:** System can detect speech start and end automatically via Voice Activity Detection (no manual trigger needed)
- **FR3:** System can transcribe English speech to text using Whisper in English-only mode
- **FR4:** System can process speech segments incrementally (transcribe as user speaks, not after full recording)

### Voice Output (Text-to-Speech)

- **FR5:** System can convert text responses to English speech with quality sufficient to sustain 30-60 minute listening sessions
- **FR6:** System can stream generated audio from server to client for playback on the tablet
- **FR7:** User can hear Claude's responses through the tablet's audio output

### Conversation Management

- **FR8:** System can invoke Claude CLI programmatically in non-interactive mode
- **FR9:** System can maintain conversation context across multiple turns within a session
- **FR10:** System can load a standalone agent definition file and pass it as system prompt to Claude CLI
- **FR11:** System can provide Claude with the contents of tracking files at session start for context awareness
- **FR12:** Claude can perform web searches during conversation without requiring user approval
- **FR13:** Claude can read and write files in the session directory (tracking `.md` files)

### Session Lifecycle

- **FR14:** User can start a session by launching the client and connecting to the server
- **FR15:** System can run a continuous voice conversation loop (listen -> transcribe -> Claude -> TTS -> play -> listen) without keyboard interaction
- **FR16:** User can pause and resume STT+TTS via a configurable hotkey
- **FR17:** User can end a session by returning to the keyboard and quitting
- **FR18:** System can retry Claude CLI requests up to 3 times on network timeout, reporting failure via audio prompt if all retries fail

### Progress Tracking

- **FR19:** System can generate a timestamped per-session synthesis file at session end (topics, errors, corrections, vocabulary, assessment)
- **FR20:** System can update a general progression document with a chronological summary of each session
- **FR21:** System can maintain a meta tracking document with overall CEFR level, NZ departure countdown, and suggested focus areas
- **FR22:** System can maintain a recurring weak points tracker listing persistent error patterns across sessions
- **FR23:** System can maintain a vocabulary journal accumulating new words and expressions with usage context
- **FR24:** System can load previous tracking files at session start to provide continuity across sessions

### Language Coaching

- **FR25:** Claude can provide real-time grammar and vocabulary corrections during conversation (default mode)
- **FR26:** User can vocally request deferred feedback mode (corrections saved for end of session or mini-session)
- **FR27:** Claude can adapt conversation vocabulary and grammar complexity based on the CEFR level recorded in the meta tracking document
- **FR28:** Claude can handle the following scenario types requested vocally: free conversation, grammar drills, interview simulation, topic discussion with web search, and level assessment — without formal mode switching
- **FR29:** Claude can conduct an initial level assessment when no previous tracking files exist
- **FR30:** Claude can suggest session focus areas based on remaining time before NZ trip and identified weak points

### Infrastructure

- **FR31:** Server can load and initialize both STT and TTS models at startup
- **FR32:** Client can connect to the server over the local network via IP and port
- **FR33:** Client can configure hotkey preference at startup
- **FR34:** Server and client can exchange bidirectional audio and control messages

## Non-Functional Requirements

### Performance

- **NFR1:** End-to-end response latency (speech end detected → audio response starts playing) must be under 5 seconds for 90% of turns
- **NFR2:** VAD silence detection must trigger within 500ms of actual speech end to avoid cutting off the user or waiting too long
- **NFR3:** TTS audio generation must begin streaming to client before full response is synthesized (streaming TTS, not batch)
- **NFR4:** STT and TTS model loading at server startup must complete within 60 seconds
- **NFR5:** Audio playback on client must start within 200ms of receiving the first TTS audio chunk

### Integration

- **NFR6:** Claude CLI invocation must support session continuity via `--continue` or `--resume` with no context loss between turns within a session
- **NFR7:** Orchestrator must handle Claude CLI response times up to 30 seconds without treating it as a failure
- **NFR8:** Audio protocol between client and server must support interleaved STT (client→server) and TTS (server→client) streams without collision or data loss
- **NFR9:** Agent definition file (`language_trainer.agent.md`) must be loadable by the orchestrator without dependency on a specific LLM backend

### Reliability

- **NFR10:** System must sustain a 60-minute continuous voice session without crashes, memory leaks, or audio pipeline degradation
- **NFR11:** Hotkey pause/resume must respond within 200ms and cleanly suspend/resume both STT and TTS pipelines
- **NFR12:** If Claude CLI becomes temporarily unreachable (network interruption), orchestrator must retry up to 3 times with 5-second intervals before reporting failure to user
- **NFR13:** Session tracking files must be written atomically — a crash during file generation must not corrupt existing tracking data
- **NFR14:** Audio pipeline must recover gracefully from transient errors (dropped packets, buffer underruns) without requiring session restart
