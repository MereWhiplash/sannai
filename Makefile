.PHONY: all build test lint fmt clean

# --- Top-level commands ---

all: build

build: build-agent build-api build-web

test: test-agent test-api test-web

lint: lint-agent lint-api lint-web

fmt: fmt-agent

clean: clean-agent clean-web

# --- Agent (Rust) ---

build-agent:
	cd agent && cargo build

test-agent:
	cd agent && cargo test

lint-agent:
	cd agent && cargo clippy -- -D warnings

fmt-agent:
	cd agent && cargo fmt

clean-agent:
	cd agent && cargo clean

# --- API (Go) ---

build-api:
	cd api && go build ./...

test-api:
	cd api && go test ./...

lint-api:
	cd api && go vet ./...

run-api:
	cd api && go run ./cmd/server

run-web:
	cd web && npm run dev

# --- Web (React) ---

build-web:
	cd web && npm run build

test-web:
	@echo "No web tests configured yet"

lint-web:
	cd web && npm run lint

typecheck-web:
	cd web && npm run typecheck

clean-web:
	rm -rf web/dist web/.output web/.nitro web/.tanstack web/node_modules

# --- Setup ---

setup:
	@echo "Installing dependencies..."
	cd web && npm install
	cd api && go mod download
	@echo "Done. Run 'make build' to build all components."
