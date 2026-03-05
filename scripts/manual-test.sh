#!/usr/bin/env bash
#
# Manual test harness for sannai.
#
# Spins up the agent with fake data, generates a session, and verifies
# it was captured correctly via the local API.
#
# Usage:
#   ./scripts/manual-test.sh          # generate + verify
#   ./scripts/manual-test.sh --live   # stream events in real-time (watch the agent process them)
#   ./scripts/manual-test.sh --keep   # don't clean up temp dirs after

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

LIVE=false
KEEP=false
for arg in "$@"; do
  case "$arg" in
    --live) LIVE=true ;;
    --keep) KEEP=true ;;
  esac
done

# Temp directories
TEST_DIR=$(mktemp -d /tmp/sannai-test.XXXXXX)
CLAUDE_DIR="$TEST_DIR/projects"
DATA_DIR="$TEST_DIR/data"
mkdir -p "$CLAUDE_DIR" "$DATA_DIR"

cleanup() {
  if [[ -n "${AGENT_PID:-}" ]] && kill -0 "$AGENT_PID" 2>/dev/null; then
    echo ""
    echo "Stopping agent (PID $AGENT_PID)..."
    kill "$AGENT_PID" 2>/dev/null || true
    wait "$AGENT_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP" == "false" ]]; then
    rm -rf "$TEST_DIR"
    echo "Cleaned up $TEST_DIR"
  else
    echo "Test data preserved at $TEST_DIR"
  fi
}
trap cleanup EXIT

echo "=== Sannai Manual Test Harness ==="
echo ""
echo "Test dir:   $TEST_DIR"
echo "Claude dir: $CLAUDE_DIR"
echo "Data dir:   $DATA_DIR"
echo ""

# --- 1. Build the agent ---
echo "--- Building agent ---"
cd "$ROOT_DIR/agent"
cargo build --quiet 2>&1
AGENT_BIN="$ROOT_DIR/agent/target/debug/sannai"
echo "Built: $AGENT_BIN"
echo ""

# --- 2. Start the agent pointing at test dirs ---
echo "--- Starting agent ---"
SANNAI_CLAUDE_DIR="$CLAUDE_DIR" \
SANNAI_DATA_DIR="$DATA_DIR" \
RUST_LOG=sannai_agent=info \
"$AGENT_BIN" start --foreground &
AGENT_PID=$!
echo "Agent PID: $AGENT_PID"

# Wait for API to be ready
echo -n "Waiting for API..."
for i in $(seq 1 30); do
  if curl -s http://127.0.0.1:9847/health > /dev/null 2>&1; then
    echo " ready!"
    break
  fi
  if ! kill -0 "$AGENT_PID" 2>/dev/null; then
    echo " FAILED (agent exited)"
    exit 1
  fi
  echo -n "."
  sleep 0.5
done
echo ""

# --- 3. Health check ---
echo "--- Health check ---"
curl -s http://127.0.0.1:9847/health | python3 -m json.tool 2>/dev/null || curl -s http://127.0.0.1:9847/health
echo ""

# --- 4. Verify no sessions yet ---
echo "--- Sessions (should be empty) ---"
SESSIONS_BEFORE=$(curl -s http://127.0.0.1:9847/sessions)
echo "$SESSIONS_BEFORE" | python3 -m json.tool 2>/dev/null || echo "$SESSIONS_BEFORE"
echo ""

# --- 5. Generate fake session ---
echo "--- Generating fake session ---"
if [[ "$LIVE" == "true" ]]; then
  bash "$SCRIPT_DIR/generate-fake-session.sh" "$CLAUDE_DIR" --live --events=3
else
  bash "$SCRIPT_DIR/generate-fake-session.sh" "$CLAUDE_DIR" --fast --events=5
fi
echo ""

# Give the watcher time to pick up the file
sleep 2

# --- 6. Verify sessions were captured ---
echo "--- Sessions (should have 1) ---"
SESSIONS=$(curl -s http://127.0.0.1:9847/sessions)
echo "$SESSIONS" | python3 -m json.tool 2>/dev/null || echo "$SESSIONS"
echo ""

SESSION_COUNT=$(echo "$SESSIONS" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "?")
if [[ "$SESSION_COUNT" == "0" || "$SESSION_COUNT" == "?" ]]; then
  echo "WARN: No sessions detected yet. The watcher may need more time."
  echo "Waiting 5 more seconds..."
  sleep 5
  SESSIONS=$(curl -s http://127.0.0.1:9847/sessions)
  echo "$SESSIONS" | python3 -m json.tool 2>/dev/null || echo "$SESSIONS"
  SESSION_COUNT=$(echo "$SESSIONS" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "?")
  echo ""
fi

# --- 7. Get session detail + events ---
SESSION_ID=$(echo "$SESSIONS" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d[0]['id'] if d else '')" 2>/dev/null || echo "")
if [[ -n "$SESSION_ID" ]]; then
  echo "--- Session detail: $SESSION_ID ---"
  DETAIL=$(curl -s "http://127.0.0.1:9847/sessions/$SESSION_ID")
  echo "$DETAIL" | python3 -m json.tool 2>/dev/null || echo "$DETAIL"
  echo ""

  echo "--- Events (first 5) ---"
  EVENTS=$(curl -s "http://127.0.0.1:9847/sessions/$SESSION_ID/events")
  EVENT_COUNT=$(echo "$EVENTS" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "?")
  echo "$EVENTS" | python3 -c '
import sys, json
events = json.load(sys.stdin)
for e in events[:5]:
    etype = e.get("event_type", "?")
    ts = e.get("timestamp", "")[:19]
    content = (e.get("content", "") or "")[:60]
    print(f"  [{etype:18s}] {ts}  {content}")
print(f"  ... {len(events)} total events")
' 2>/dev/null || echo "  ($EVENT_COUNT events)"
  echo ""
fi

# --- 8. Test commit hook ---
# NOTE: Currently skipped — the session manager holds the SessionManager
# mutex for its entire lifetime, which blocks the commit hook handler.
# See GitHub issue for the lock contention fix.
echo "--- Commit hook: SKIPPED (lock contention bug — see issues) ---"
echo ""

# --- 9. Summary ---
echo "=== Test Summary ==="
echo "Sessions captured: $SESSION_COUNT"
if [[ -n "$SESSION_ID" ]]; then
  echo "Session ID:        $SESSION_ID"
  echo "Events captured:   ${EVENT_COUNT:-?}"
fi
echo ""
echo "SQLite DB:         $DATA_DIR/store.db"
echo ""
echo "You can inspect the database manually:"
echo "  sqlite3 $DATA_DIR/store.db '.tables'"
echo "  sqlite3 $DATA_DIR/store.db 'SELECT * FROM sessions;'"
echo "  sqlite3 $DATA_DIR/store.db 'SELECT * FROM events LIMIT 10;'"
echo "  sqlite3 $DATA_DIR/store.db 'SELECT * FROM commit_links;'"

if [[ "$KEEP" == "true" ]]; then
  echo ""
  echo "Agent still running (PID $AGENT_PID). Press Ctrl+C to stop."
  wait "$AGENT_PID" 2>/dev/null || true
fi
