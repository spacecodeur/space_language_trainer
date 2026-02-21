# --- Config (override via environment or command line) ---
WHISPER_MODEL   ?= small
TTS_MODEL       ?= $(HOME)/models/kokoro-en-v0_19
TTS_LANG        ?= en
SERVER_PORT     ?= 9500
SOCKET_PATH     ?= /tmp/space_lt_server.sock
AGENT           ?= agent/language_trainer.agent.md
SERVER_ADDR     ?= 127.0.0.1:$(SERVER_PORT)
CUDA            ?= 1
DEBUG           ?= 0

ifeq ($(DEBUG),1)
  DEBUG_FLAG = --debug
else
  DEBUG_FLAG =
endif

# CUDA=1 enables GPU for Whisper (STT) only — TTS stays on CPU (sherpa-rs CUDA crashes).
# Use CUDA=all to force both Whisper + TTS on GPU, or CUDA=0 for full CPU.
ifeq ($(CUDA),all)
  SERVER_FEATURES = --features cuda-all
else ifeq ($(CUDA),1)
  SERVER_FEATURES = --features cuda
else
  SERVER_FEATURES =
endif

.PHONY: build check test test-common test-server test-orchestrator test-client \
        run-server run-orchestrator run-client

# --- Build ---

build:
	cargo build --workspace

# --- Quality ---

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

# --- Run ---

run-server:
	cargo run -p space_lt_server $(SERVER_FEATURES) -- \
		--model $(WHISPER_MODEL) \
		--tts-model $(TTS_MODEL) \
		--language $(TTS_LANG) \
		--port $(SERVER_PORT) \
		--socket-path $(SOCKET_PATH) \
		$(DEBUG_FLAG)

run-orchestrator:
	cargo run -p space_lt_orchestrator -- \
		--agent $(AGENT) \
		--socket $(SOCKET_PATH) \
		$(DEBUG_FLAG)

run-client:
	cargo run -p space_lt_client -- \
		--server $(SERVER_ADDR) \
		$(DEBUG_FLAG)
