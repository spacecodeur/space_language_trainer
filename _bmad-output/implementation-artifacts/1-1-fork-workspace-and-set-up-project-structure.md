# Story 1.1: Fork Workspace and Set Up Project Structure

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want to fork `space_tts` into an independent `space_language_training` workspace with 4 crates,
so that I have a clean development foundation without breaking the original STT project.

## Acceptance Criteria

1. **Given** the existing `space_tts` repository at `/home/okli/code/space_tts/`
   **When** the developer copies its source into `space_language_training`
   **Then** the workspace contains 4 crates: `common`, `client`, `server`, `orchestrator`

2. **Given** the new workspace
   **When** examining `orchestrator/src/main.rs`
   **Then** it contains a minimal hello-world entry point

3. **Given** the workspace root `Cargo.toml`
   **When** examining its `[workspace]` section
   **Then** `members` includes all 4 crates: `["common", "client", "server", "orchestrator"]`

4. **Given** the project root
   **When** examining the `Makefile`
   **Then** it provides targets: `build`, `check`, `test`, `test-common`, `test-server`, `test-orchestrator`, `test-client`

5. **Given** the complete workspace
   **When** running `make build`
   **Then** it succeeds without errors

6. **Given** the complete workspace
   **When** running `make check`
   **Then** it passes (fmt + clippy + test)

## Tasks / Subtasks

- [x] Task 1: Copy space_tts source into space_language_training (AC: #1)
  - [x] 1.1: Copy `common/`, `client/`, `server/` source directories from `/home/okli/code/space_tts/`
  - [x] 1.2: Copy `Cargo.lock` from space_tts (preserves dependency versions)
  - [x] 1.3: Copy `setup.sh` from space_tts
  - [x] 1.4: Do NOT copy `target/`, `_bmad/`, `_bmad-output/`, `docs/`, `README.md` (already exist or not needed)

- [x] Task 2: Rename packages from space_tts to space_lt (AC: #1, #3)
  - [x] 2.1: Create root `Cargo.toml` with `workspace.members = ["common", "client", "server", "orchestrator"]` and `resolver = "3"`
  - [x] 2.2: Rename `common/Cargo.toml` package name: `space_tts_common` → `space_lt_common`
  - [x] 2.3: Rename `client/Cargo.toml` package name: `space_tts_client` → `space_lt_client`
  - [x] 2.4: Update client dependency: `space_tts_common` → `space_lt_common = { path = "../common" }`
  - [x] 2.5: Rename `server/Cargo.toml` package name: `space_tts_server` → `space_lt_server`
  - [x] 2.6: Update server dependency: `space_tts_common` → `space_lt_common = { path = "../common" }`
  - [x] 2.7: Update all `use space_tts_common::` → `use space_lt_common::` in client and server source files

- [x] Task 3: Create orchestrator crate (AC: #2)
  - [x] 3.1: Create `orchestrator/Cargo.toml` with package name `space_lt_orchestrator`, edition `"2024"`, dependency on `space_lt_common = { path = "../common" }` and `anyhow = "1.0.101"`
  - [x] 3.2: Create `orchestrator/src/main.rs` with minimal hello-world: `fn main() { println!("space_lt_orchestrator"); }`

- [x] Task 4: Create Makefile (AC: #4)
  - [x] 4.1: Create `Makefile` at project root with all required targets (see Dev Notes for exact content)

- [x] Task 5: Verify build and checks (AC: #5, #6)
  - [x] 5.1: Run `make build` — must succeed
  - [x] 5.2: Run `make check` — must pass (fmt + clippy + test)
  - [x] 5.3: Run `make test` — all existing tests must pass

## Dev Notes

### Source Codebase Location

The existing `space_tts` codebase to fork is located at: `/home/okli/code/space_tts/`

Current structure:
```
space_tts/
├── Cargo.toml              # workspace: common, client, server (resolver = "3")
├── Cargo.lock
├── setup.sh
├── common/
│   ├── Cargo.toml          # space_tts_common, deps: anyhow 1.0.101
│   └── src/
│       ├── lib.rs
│       ├── log.rs           # info!/debug!/warn! macros
│       ├── models.rs        # model path resolution
│       └── protocol.rs      # ClientMsg/ServerMsg, 9 round-trip tests
├── client/
│   ├── Cargo.toml          # space_tts_client
│   └── src/
│       ├── main.rs          # entry point
│       ├── audio.rs         # cpal capture + resampling
│       ├── hotkey.rs        # evdev monitoring
│       ├── inject.rs        # audio injection utility
│       ├── remote.rs        # SSH-based communication (will be replaced by TCP in Story 2.4)
│       ├── tui.rs           # ratatui setup wizard
│       └── vad.rs           # voice activity detection
└── server/
    ├── Cargo.toml          # space_tts_server, features: cuda
    └── src/
        ├── main.rs          # entry point
        ├── server.rs        # SSH-based server logic
        └── transcribe.rs    # Whisper STT integration
```

### Package Naming Convention

Architecture specifies hyphenated names in Cargo.toml:
- `space_lt_common` (package name uses underscores per Rust convention)
- But Makefile targets reference `-p space-lt-common` (Cargo uses the package name)

**IMPORTANT:** Rust package names in Cargo.toml use underscores (`space_lt_common`). Cargo's `-p` flag accepts either form, but be consistent. Use underscore in Cargo.toml, the `-p` flag will accept `space_lt_common`.

### Existing Protocol (DO NOT MODIFY in this story)

The protocol in `common/src/protocol.rs` currently has:
- `ClientMsg::AudioSegment` (0x01)
- `ServerMsg::Ready` (0x80), `ServerMsg::Text` (0x81), `ServerMsg::Error` (0x82)
- 9 existing round-trip tests

Protocol extension happens in Story 1.3 — this story only renames packages.

### Makefile Content

The architecture specifies these exact targets:

```makefile
.PHONY: build check test test-common test-server test-orchestrator test-client

build:
	cargo build --workspace

check:
	cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace

test:
	cargo test --workspace

test-common:
	cargo test -p space_lt_common

test-server:
	cargo test -p space_lt_server

test-orchestrator:
	cargo test -p space_lt_orchestrator

test-client:
	cargo test -p space_lt_client
```

**Note:** Architecture also mentions `run-server`, `run-orchestrator`, `run-client` targets but those require CLI arguments that don't exist yet. Add only the required targets for this story.

### CLAUDE.md Global Rule

When a Makefile exists, ALWAYS use Makefile targets instead of raw commands. After creating the Makefile, use `make check` instead of `cargo fmt && cargo clippy && cargo test`.

### Project Structure Notes

- The `space_language_training` repo already has: `README.md`, `_bmad/`, `_bmad-output/`, `docs/`
- These must NOT be overwritten or deleted during the fork
- The `.git` directory belongs to space_language_training — do NOT copy space_tts's .git
- `setup.sh` from space_tts should be copied (will be extended later for TTS model download)

### Edition and Resolver

Both space_tts and this project use Rust Edition 2024 with `resolver = "3"`. Maintain this.

### Dependencies to Preserve Exactly

| Crate | Dependency | Version |
|-------|-----------|---------|
| common | anyhow | 1.0.101 |
| client | space_lt_common | path = "../common" |
| client | anyhow | 1.0.101 |
| client | audioadapter-buffers | 2.0.0 |
| client | cpal | 0.17.3 |
| client | crossbeam-channel | 0.5.15 |
| client | crossterm | 0.29.0 |
| client | ctrlc | 3.5.2 (features: termination) |
| client | evdev | 0.13.2 |
| client | ratatui | 0.30.0 |
| client | rubato | 1.0.1 |
| client | webrtc-vad | 0.4.0 |
| server | space_lt_common | path = "../common" |
| server | whisper-rs | 0.15.1 (features: cuda optional) |
| server | anyhow | 1.0.101 |
| orchestrator | space_lt_common | path = "../common" |
| orchestrator | anyhow | 1.0.101 |

### References

- [Source: architecture.md#Workspace Structure] — 4-crate layout, crate names
- [Source: architecture.md#Development Workflow] — Makefile targets
- [Source: architecture.md#Naming Conventions] — snake_case packages
- [Source: epics.md#Story 1.1] — Acceptance criteria
- [Source: architecture.md#Core Architectural Decisions] — Workspace fork decision and rationale

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

None

### Completion Notes List

- Copied common/, client/, server/, Cargo.lock, setup.sh from space_tts
- Renamed all packages from space_tts_* to space_lt_*
- Updated all `use space_tts_common::` imports across 8 source files
- Updated binary name references (space_tts_server → space_lt_server) in server/src/main.rs and client/src/remote.rs
- Created orchestrator crate with minimal main.rs
- Created Makefile with all required targets
- Fixed 2 clippy issues inherited from space_tts (collapsible_if and manual_is_multiple_of in server/src/transcribe.rs)
- `make check` passes: fmt clean, clippy clean (-D warnings), 33 tests pass

**Code review fixes (adversarial review):**
- Added `.gitignore` with `/target` (HIGH: build artifacts would be committed)
- Updated `setup.sh`: all 16 `space_tts_*` references → `space_lt_*` (MEDIUM: script was broken)
- Updated `common/src/models.rs`: data directory `space_tts` → `space_lt` (MEDIUM: shared data dir)
- Updated test temp directory names in `common/src/models.rs` (LOW)

### File List

- `.gitignore` — new (review fix)
- `Cargo.toml` — workspace root (new)
- `Cargo.lock` — copied from space_tts
- `Makefile` — build targets (new)
- `setup.sh` — copied from space_tts, renamed space_tts → space_lt references (review fix)
- `common/Cargo.toml` — renamed package
- `common/src/lib.rs` — copied
- `common/src/log.rs` — copied
- `common/src/models.rs` — copied, updated data dir path + test names (review fix)
- `common/src/protocol.rs` — copied
- `client/Cargo.toml` — renamed package + dependency
- `client/src/main.rs` — updated imports + binary name
- `client/src/audio.rs` — updated imports
- `client/src/hotkey.rs` — updated imports
- `client/src/inject.rs` — updated imports
- `client/src/remote.rs` — updated imports + binary name
- `client/src/tui.rs` — copied
- `client/src/vad.rs` — copied
- `server/Cargo.toml` — renamed package + dependency
- `server/src/main.rs` — updated imports + binary name
- `server/src/server.rs` — updated imports
- `server/src/transcribe.rs` — updated imports + clippy fixes
- `orchestrator/Cargo.toml` — new crate
- `orchestrator/src/main.rs` — new minimal entry point
