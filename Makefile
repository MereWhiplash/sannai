.PHONY: all build test lint fmt clean

all: build

build:
	cd agent && cargo build

test:
	cd agent && cargo test

lint:
	cd agent && cargo clippy -- -D warnings

fmt:
	cd agent && cargo fmt

clean:
	cd agent && cargo clean

# --- Manual Testing ---

manual-test:
	./scripts/manual-test.sh

manual-test-live:
	./scripts/manual-test.sh --live --keep
