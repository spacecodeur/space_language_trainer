---
stepsCompleted: [1, 2, 3, 4, 5, 6]
status: complete
inputDocuments: []
date: 2026-02-20
author: Matthieu
---

# Product Brief: space_language_training

## Executive Summary

Space Language Training extends the existing `space_tts` Rust project into a voice-based English practice platform. It combines the proven STT client/server architecture (Whisper on GPU) with a new TTS engine and a Claude CLI orchestrator to create hands-free conversational English training sessions. The system runs as a client/server setup over the local network: the tablet captures and plays audio, while the desktop machine handles speech processing and Claude CLI interaction. Session tracking via markdown files provides progression visibility. No additional API costs beyond the existing Claude CLI subscription.

---

## Core Vision

### Problem Statement

Developers and desk workers who want to improve their spoken English lack a practical, hands-free conversational practice tool that fits into their daily routines. Existing language apps focus on reading/writing or require manual interaction, making them incompatible with physical activities like treadmill walking.

### Problem Impact

- Wasted time during physical exercise that could be used productively
- Spoken English skills stagnate without regular conversational practice
- A2/B1 learners need consistent, varied oral practice to progress — not grammar drills or flashcards
- Preparing for travel or professional contexts (interviews, meetings) requires realistic conversational simulation

### Why Existing Solutions Fall Short

- **Duolingo, Babbel, etc.** — Screen-dependent, focused on reading/writing, no real conversation
- **Companion, Discute** (open-source) — Tied to OpenAI/Groq APIs (additional costs), no client/server architecture, limited session tracking
- **RealtimeVoiceChat** — Good voice architecture but no language learning features, no longer maintained
- **AgentVibes** — TTS-only add-on for Claude CLI, does not handle STT or conversation
- **Human tutors** — Expensive, scheduling constraints, not available on-demand during exercise

### Proposed Solution

Extend `space_tts` (existing Rust STT client/server) into a complete voice conversation platform:

1. **Add TTS** to the server (Piper or Kokoro) for Claude's spoken responses
2. **Build an orchestrator** that loops: listen → STT → Claude CLI → TTS → speak → listen
3. **Create a Claude CLI agent** (`language_trainer`) with language teaching persona, feedback modes, and scenario support
4. **Implement session tracking** via `.md` synthesis files with cross-session progression
5. **Leverage Claude CLI** programmatic mode (`claude -p`, `--continue`) for zero additional API cost

Supports diverse scenarios: free conversation, grammar drills, fluency exercises, interview simulation, level assessment, topic-based discussion with web research.

### Key Differentiators

- **Zero additional cost** — Runs entirely on existing Claude CLI subscription + local hardware
- **Truly hands-free** — VAD-based turn detection, no keyboard interaction needed during sessions
- **Built on proven foundation** — Extends battle-tested STT client/server architecture
- **Claude's intelligence** — Unlike local LLM solutions, benefits from Claude's advanced language teaching, correction, and conversational abilities
- **Session continuity** — Markdown-based progression tracking across sessions with synthesized learning insights
- **Full English immersion** — Whisper locked to English mode, pushing the learner to stay in the target language at all times

---

## Target Users

### Primary User

**Matthieu — Développeur informaticien, niveau anglais A2/B1**

- **Contexte :** Développeur passant l'essentiel de ses journées assis. Utilise un tapis roulant pour compenser la sédentarité. Prépare un voyage en Nouvelle-Zélande (mai 2026) et veut progresser en anglais oral.
- **Environnement technique :** Machine fixe puissante (NVIDIA 4080/16Go, Fedora) et tablette portable (Fedora). Abonné Claude CLI. Développeur Rust expérimenté.
- **Motivation :** Rentabiliser le temps de marche en pratiquant l'anglais conversationnel. Progresser de A2/B1 vers B2+ avant le voyage.
- **Frustrations actuelles :** Les apps existantes (Duolingo, etc.) sont orientées lecture/écriture, nécessitent les mains, et ne proposent pas de vraie conversation adaptative.
- **Ce qui le ferait dire "c'est exactement ce qu'il me fallait" :** Voir le système suivre sa progression au fil des sessions et constater des progrès mesurables en quelques semaines.

### Secondary Users

N/A — Projet personnel. Ciblé exclusivement sur un écosystème Linux/Fedora (client et serveur).

### User Journey

**1. Lancement**
- Matthieu monte sur le tapis roulant, le démarre
- Ouvre un terminal sur sa tablette, lance Claude CLI
- Invoque l'agent `/space_language_trainer`, pose le clavier
- Le système passe en mode vocal : STT + TTS actifs, prêt à converser

**2. Session active (30-60 min)**
- Conversation entièrement orale, mains libres
- Scénarios variés : conversation libre, exercices de grammaire, simulation d'entretien, discussion sur un sujet d'actualité avec recherche web, etc.
- Feedback par défaut en temps réel (corrections immédiates), avec possibilité de demander vocalement un feedback différé (en fin de mini-session)
- Hotkey disponible pour pause/reprise STT+TTS (interruptions de la vie quotidienne)

**3. Fin de session**
- Phase de synthèse orale avec l'agent (bilan de la session, points travaillés, axes de progression)
- Matthieu reprend le clavier et quitte Claude CLI (ou commande vocale de fin)
- Un fichier `.md` de synthèse est automatiquement généré
- Le document de progression global est mis à jour

**4. Inter-sessions**
- À chaque nouvelle session, l'agent consulte les synthèses précédentes pour reprendre là où on s'est arrêté
- Progression visible via les fichiers `.md` (historique, points forts/faibles, évolution)

---

## Success Metrics

### User Success Metrics (tracked by AI agent via session `.md` files)

- **Regularity:** Consistent daily usage (target: 5+ sessions/week, 30-60 min each)
- **Conversation fluency:** Ability to sustain English-only conversation without prolonged pauses or French fallback, measured by session duration in continuous English
- **Grammar accuracy progression:** Reduction in recurring grammar errors tracked across sessions (verb tenses, articles, prepositions)
- **Vocabulary expansion:** New words and expressions used correctly, tracked session over session
- **Scenario diversity:** Ability to handle increasingly complex scenarios (from free conversation to interview simulation)
- **CEFR level progression:** From A2/B1 toward B2/C1, assessed periodically via dedicated evaluation sessions

### Technical Success Metrics

- **STT accuracy:** Whisper transcription quality sufficient to detect grammatical and vocabulary errors reliably (target: large model, English-only mode)
- **TTS naturalness:** Voice quality realistic enough to train listening comprehension for real-world conversations (target: Orpheus TTS or equivalent high-quality model). Also serves as auditory pronunciation reference — hearing correct pronunciation helps self-correction.
- **Pronunciation feedback:** Deferred to future enhancement — requires phoneme-level analysis beyond Whisper's word-level transcription. Not in MVP scope. Natural TTS provides passive pronunciation reference in the meantime.
- **Latency:** Acceptable conversational pace. Model selection (Whisper model size, TTS model) is the primary lever — to be tuned during implementation.

### Business Objectives

N/A — Personal project. Success is measured entirely by English proficiency improvement and tool usability.

### Key Performance Indicators

| KPI | Target | Timeframe |
|-----|--------|-----------|
| Sessions completed | 5+/week | Ongoing |
| Average session duration | 30-60 min | From week 1 |
| Recurring grammar errors | Decreasing trend | Monthly review |
| CEFR self-assessment | B1 solid | Month 1 |
| CEFR self-assessment | B2 | Month 3 (May 2026) |
| Scenario complexity handled | Interview simulation, debate | Month 2 |

---

## MVP Scope

### Core Features

**1. Voice Conversation Loop (extends `space_tts`)**
- Add TTS engine to server (Orpheus or equivalent high-quality model)
- Bidirectional audio streaming: tablet (capture/playback) ↔ desktop (STT/TTS processing)
- Orchestrator loop: listen (VAD) → Whisper STT → Claude CLI → TTS → audio playback → listen
- Hotkey pause/resume for STT+TTS (interrupt handling)

**2. Claude CLI Integration**
- Programmatic invocation of Claude CLI (`claude -p`, `--continue`/`--resume`) for conversation management within existing subscription
- Agent `language_trainer` with English coaching persona, CEFR-aware methodology, real-time feedback by default, deferred feedback on request
- Web search enabled without user approval — allows discussing current events and researching topics mid-conversation
- Adaptive scenario handling: no formal scenario system, Claude adapts to whatever the user requests vocally (free conversation, grammar drills, interview simulation, etc.)

**3. Session Tracking System (`.md` files)**
- **Per-session synthesis** — timestamped file generated at end of each session: topics covered, errors made, corrections given, vocabulary learned, session assessment
- **General progression document** — chronological accumulation of session-by-session summaries, updated after each session
- **Meta tracking document** — overall English level (CEFR), NZ departure countdown (May 2026), suggested focus areas based on remaining time, milestones achieved
- **Recurring weak points tracker** — persistent list of error patterns across sessions (e.g., "confuses simple past / present perfect"), updated when errors recur or are resolved
- **Vocabulary journal** — cumulative list of new words and expressions learned per session, with context of usage

**4. Session Lifecycle**
- Start: orchestrator loads meta document + recent session syntheses + weak points tracker into Claude context
- Active: hands-free voice conversation with real-time feedback
- End: Claude generates session synthesis, updates progression document, weak points tracker, and vocabulary journal

### Out of Scope for MVP

- **Pronunciation analysis** — phoneme-level detection (e.g., silent K in "know") requires specialized models beyond Whisper. Deferred to future enhancement.
- **Multiple TTS voices/accents** — MVP ships with one high-quality English voice. Multiple accents (British, Australian, NZ) deferred.
- **GUI/dashboard** — progression is tracked in `.md` files, no visual dashboard. Files are human-readable and can be consulted manually.
- **Multi-platform support** — Linux/Fedora only (client and server). No macOS, Windows, Android, iOS.
- **Multi-user support** — single user, single configuration.
- **Offline LLM fallback** — no local LLM. If internet is unavailable, the application does not function (STT/TTS still work but conversation requires Claude API).

### MVP Success Criteria

- Complete a 30-minute hands-free English conversation session end-to-end
- Session synthesis `.md` file generated automatically at session end
- Next session correctly loads previous context and adapts accordingly
- Weak points tracker accurately reflects recurring errors across 5+ sessions
- Whisper transcription quality sufficient for Claude to detect and correct grammar/vocabulary errors
- TTS quality natural enough to serve as pronunciation reference
- Latency acceptable for natural conversational pace

### Future Vision

- **Pronunciation coaching** — phoneme-level analysis with targeted repetition exercises
- **Multiple voices and accents** — NZ English, British, American, Australian accents to train ear
- **Structured curriculum** — CEFR-aligned lesson plans with formal level assessments
- **Progress dashboard** — web-based visualization of progression over time
- **Conversation history replay** — re-listen to past sessions to hear own progression
- **Spaced repetition** — integrate vocabulary journal with spaced repetition algorithm for optimal retention
- **Multi-language support** — extend beyond English to other target languages
