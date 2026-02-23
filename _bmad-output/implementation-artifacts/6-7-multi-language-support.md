# Story 6.7: Multi-Language Support

Status: ready-for-dev

## Story

As a **language learner**,
I want to choose between English, Mandarin, Japanese, Spanish, or Hindi at session start,
So that I can practice multiple languages using the same app.

## Context

The app currently only supports English: the TTS model is `kokoro-en-v0_19` (English-only, 11 voices), the agent prompt is English-specific, and there is no language selection UI. However, the architecture is already partially prepared for multi-language: `--language` flows from Makefile to server to Whisper + TTS, and the protocol's SessionStart carries a JSON config.

### Model Verification

**Kokoro `multi-lang-v1_1`:**
- Size: model.onnx ~326 MB, total directory ~400 MB (voices.bin + tokens + espeak-ng-data + lexicons)
- RAM: ~1.5 GB (comparable to current en-v0_19)
- Parameters: 82M (same as en-v0_19, just more speaker embeddings)
- 103 voices across 9 languages
- CPU performance: on Raspberry Pi 4 it runs 3.2x real-time — on a desktop PC it will be near-instant
- GPU: CPU-only is fine for this model size (sherpa-rs CUDA still unreliable)

**Voice mapping for target languages:**

| Language | Code | Whisper | Kokoro voices | Default voice | Speaker ID |
|----------|------|---------|---------------|---------------|:---:|
| English (US) | `en` | Excellent | 20 (af/am) | `af_heart` | 3 |
| Mandarin | `zh` | Good | 8 (zf/zm) | `zf_xiaoxiao` | 47 |
| Japanese | `ja` | Good | 5 (jf/jm) | `jf_alpha` | 37 |
| Spanish | `es` | Good | 3 (ef/em) | `ef_dora` | 28 |
| Hindi | `hi` | Good | 4 (hf/hm) | `hf_alpha` | 31 |

**Whisper support:** All 5 languages have good STT quality. Hindi initial prompt is currently missing in `transcribe.rs` (the other 4 are present).

**Claude support:** Excellent for EN/ES, good for ZH/JA/HI. Claude can coach in all 5 languages.

## Acceptance Criteria

1. **Given** the client starts
   **When** the TUI setup wizard runs
   **Then** it presents a language selection menu: English, Mandarin, Japanese, Spanish, Hindi
   **And** the chosen language is stored in the session config

2. **Given** the user selects a language
   **When** the client connects to the server
   **Then** the language is passed in the SessionStart JSON config
   **And** the server uses this language for Whisper STT (transcription bias)
   **And** the server selects the appropriate TTS voice for that language
   **And** the orchestrator loads the corresponding agent prompt file

3. **Given** a Mandarin session
   **When** the user speaks Mandarin
   **Then** Whisper transcribes in Mandarin, Claude responds in Mandarin, TTS speaks Mandarin with a native Chinese voice

4. **Given** the TTS model
   **When** the server starts
   **Then** it uses `kokoro-multi-lang-v1_1` (supports all 5 languages from a single model)
   **And** the model loads once at startup, language selection only changes the speaker_id per session

5. **Given** each target language
   **When** the orchestrator loads the agent prompt
   **Then** a dedicated agent file exists per language with:
   - Persona adapted to the language (tutor personality, cultural references)
   - CEFR methodology adapted (or equivalent framework for non-European languages)
   - Correction examples in the target language
   - Scenario prompts in the target language

6. **Given** the complete system
   **When** running `make check`
   **Then** all existing + new tests pass with zero warnings

## Tasks / Subtasks

- [ ] Task 1: TTS multi-lang model + voice selection
  - [ ] 1.1: Update Makefile `TTS_MODEL` default to `kokoro-multi-lang-v1_1`
  - [ ] 1.2: Add `speaker_id_for_language(lang: &str) -> i32` mapping function in `tts.rs`
  - [ ] 1.3: Accept `speaker_id` as parameter in `KokoroTts::new()` instead of hardcoding 0
  - [ ] 1.4: Server resolves speaker_id from language before constructing TTS engine
  - [ ] 1.5: Unit tests for speaker_id mapping

- [ ] Task 2: Client language selection
  - [ ] 2.1: Add language selection step in TUI setup (`tui::run_setup()`)
  - [ ] 2.2: Store chosen language in client config
  - [ ] 2.3: Include language in the TCP connection config (sent during handshake)
  - [ ] 2.4: Update `run_client()` to pass language through the system

- [ ] Task 3: Protocol & server language routing
  - [ ] 3.1: Parse `language` field from ClientMsg config or SessionStart JSON
  - [ ] 3.2: Server: override default language with client-provided language for this session
  - [ ] 3.3: Forward language to orchestrator in OrchestratorMsg::SessionStart config JSON
  - [ ] 3.4: Whisper: add Hindi initial prompt (`"hi" => "नमस्ते, यह हिंदी ट्रांस्क्रिप्शन है।"`)
  - [ ] 3.5: Set Whisper language dynamically per session (already parameterized in `params.set_language()`)

- [ ] Task 4: Agent prompts per language
  - [ ] 4.1: Create `agent/language_trainer_zh.agent.md` — Mandarin tutor persona
  - [ ] 4.2: Create `agent/language_trainer_ja.agent.md` — Japanese tutor persona
  - [ ] 4.3: Create `agent/language_trainer_es.agent.md` — Spanish tutor persona
  - [ ] 4.4: Create `agent/language_trainer_hi.agent.md` — Hindi tutor persona
  - [ ] 4.5: Rename current `language_trainer.agent.md` → `language_trainer_en.agent.md`
  - [ ] 4.6: Each agent file includes: persona, CEFR/equivalent levels, correction techniques, example corrections, scenario triggers, feedback format, context compaction instructions, session summary awareness

- [ ] Task 5: Orchestrator language-aware agent loading
  - [ ] 5.1: Parse language from SessionStart config JSON
  - [ ] 5.2: Resolve agent file path: `agent/language_trainer_{lang}.agent.md`
  - [ ] 5.3: Update Makefile `AGENT` variable to be a directory or template (not a single file)
  - [ ] 5.4: FORMAT_REMINDER: keep language-agnostic (it's about TTS output format, not language content)
  - [ ] 5.5: SUMMARY_PROMPT: keep language-agnostic (Claude knows the session language from context)

- [ ] Task 6: Validation
  - [ ] 6.1: `make check` — all tests pass, zero warnings
  - [ ] 6.2: Download `kokoro-multi-lang-v1_1` model, verify it loads
  - [ ] 6.3: E2E test per language: select language → speak → hear response in correct language
  - [ ] 6.4: Verify voice quality is acceptable for each language
  - [ ] 6.5: Verify session summary works correctly for non-English sessions

## Dev Notes

### TTS Model Switch

Current: `~/models/kokoro-en-v0_19` (English only, 11 voices, ~300 MB)
Target: `~/models/kokoro-multi-lang-v1_1` (9 languages, 103 voices, ~400 MB)

The multi-lang model is only ~100 MB larger. It's a drop-in replacement — same ONNX format, same sherpa-rs API. The difference is more speaker embeddings in `voices.bin` and additional lexicon files.

Download: `setup.sh` should be updated to download multi-lang instead of en-only.

### Speaker ID Resolution

The speaker_id selects which voice embedding to use. The mapping must be maintained as a simple lookup:

```rust
fn speaker_id_for_language(lang: &str) -> i32 {
    match lang {
        "en" => 3,   // af_heart (American English female)
        "zh" => 47,  // zf_xiaoxiao (Chinese female)
        "ja" => 37,  // jf_alpha (Japanese female)
        "es" => 28,  // ef_dora (Spanish female)
        "hi" => 31,  // hf_alpha (Hindi female)
        _ => 3,      // fallback to English
    }
}
```

### Language Flow

```
Client TUI → language choice
  → TCP connect
  → Client includes "language": "ja" in session config
  → Server receives, updates Whisper language + TTS speaker_id
  → Server forwards language in SessionStart JSON to orchestrator
  → Orchestrator loads agent/language_trainer_ja.agent.md
  → Session proceeds entirely in Japanese
```

### Agent File Strategy

One agent file per language rather than a template system. Reasons:
- Each language has unique grammar rules, common errors, cultural context
- CEFR applies to European languages but not directly to Mandarin/Japanese/Hindi
- Correction examples must be authentic per language
- Simpler to maintain than a complex templating system
- Claude can be asked to help draft the initial agent files

### What Stays Language-Agnostic

- **FORMAT_REMINDER**: Instructions about TTS output format (no markdown, short sentences, feedback blocks) — these apply to ALL languages
- **SUMMARY_PROMPT**: Asks for vocabulary/errors/grammar recap — Claude knows the session language from context and will produce the summary in the appropriate language
- **Protocol**: No protocol changes needed — SessionStart JSON already carries config, just add a "language" field
- **Audio pipeline**: 16kHz mono i16 everywhere — language-independent

### Whisper Multi-Language

Whisper supports all 5 target languages natively. The `language` parameter biases the model toward a specific language (otherwise it auto-detects, which adds latency and can be wrong for short utterances). The `initial_prompt` function already has entries for 4/5 languages — only Hindi is missing.

### Files to Modify

- `Makefile` — TTS_MODEL default, AGENT handling
- `setup.sh` — download multi-lang model
- `server/src/tts.rs` — speaker_id_for_language(), parameterize constructor
- `server/src/main.rs` — resolve speaker_id from language
- `server/src/transcribe.rs` — add Hindi initial prompt
- `server/src/session.rs` — parse language from client config, forward to orchestrator
- `client/src/main.rs` — language selection in TUI
- `orchestrator/src/main.rs` — resolve agent file from language
- `orchestrator/src/voice_loop.rs` — accept dynamic agent path
- `agent/language_trainer_en.agent.md` — rename from language_trainer.agent.md
- `agent/language_trainer_zh.agent.md` — new
- `agent/language_trainer_ja.agent.md` — new
- `agent/language_trainer_es.agent.md` — new
- `agent/language_trainer_hi.agent.md` — new
