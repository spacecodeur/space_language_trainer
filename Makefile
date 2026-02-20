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
