#!/usr/bin/env bash
#
# Generate a fake Claude Code JSONL session for manual testing.
#
# Usage:
#   ./scripts/generate-fake-session.sh [target-dir]
#
# If target-dir is omitted, writes to /tmp/sannai-test-data/projects/
# The script creates the directory structure the watcher expects:
#   <target-dir>/<project-slug>/<session-uuid>.jsonl
#
# Options:
#   --live    Write events one-by-one with delays (simulates real-time session)
#   --fast    Write all events at once (default)
#   --events  Number of prompt/response pairs to generate (default: 5)

set -euo pipefail

TARGET_DIR="${1:-/tmp/sannai-test-data/projects}"
MODE="fast"
NUM_EVENTS=5

# Parse flags
for arg in "$@"; do
  case "$arg" in
    --live)  MODE="live" ;;
    --fast)  MODE="fast" ;;
    --events=*) NUM_EVENTS="${arg#--events=}" ;;
  esac
done

SESSION_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
PROJECT_SLUG="-Users-test-dev-myproject"
PROJECT_DIR="$TARGET_DIR/$PROJECT_SLUG"
SESSION_FILE="$PROJECT_DIR/$SESSION_ID.jsonl"

mkdir -p "$PROJECT_DIR"

echo "Session ID:   $SESSION_ID"
echo "Project dir:  $PROJECT_DIR"
echo "Session file: $SESSION_FILE"
echo "Mode:         $MODE ($NUM_EVENTS prompt/response pairs)"
echo ""

# Helper: ISO timestamp offset by N seconds from base
BASE_TS=$(date -u +%s)
ts_offset() {
  local offset=$1
  if [[ "$(uname)" == "Darwin" ]]; then
    date -u -r $((BASE_TS + offset)) +"%Y-%m-%dT%H:%M:%S.000Z"
  else
    date -u -d "@$((BASE_TS + offset))" +"%Y-%m-%dT%H:%M:%S.000Z"
  fi
}

write_line() {
  echo "$1" >> "$SESSION_FILE"
  if [[ "$MODE" == "live" ]]; then
    sleep 1
  fi
}

# Sample prompts and responses for realistic sessions
PROMPTS=(
  "Add error handling to the upload function"
  "Write unit tests for the auth middleware"
  "Refactor the database connection pool to use lazy initialization"
  "Fix the race condition in the session manager"
  "Add pagination to the list endpoint"
  "Implement retry logic for failed API calls"
  "Create a Dockerfile for the web service"
  "Add input validation for the user registration form"
  "Optimize the SQL query that loads dashboard data"
  "Add structured logging with request IDs"
)

RESPONSES=(
  "I'll add try-catch blocks around the upload logic and return meaningful error messages to the caller."
  "I'll create a test suite covering the happy path, invalid tokens, expired tokens, and missing auth headers."
  "I'll wrap the connection pool in a OnceCell so it's initialized on first use rather than at startup."
  "The race condition is caused by concurrent access to the sessions map. I'll switch to a DashMap for lock-free reads."
  "I'll add limit/offset query params and return pagination metadata in the response headers."
  "I'll implement exponential backoff with jitter, capped at 3 retries, using the reqwest middleware pattern."
  "I'll create a multi-stage Dockerfile with a builder stage for compilation and a slim runtime image."
  "I'll add Zod schema validation on the server function and client-side validation in the form component."
  "The N+1 query can be replaced with a single JOIN. I'll also add an index on the session_id foreign key."
  "I'll integrate the tracing crate with a JSON formatter and propagate request IDs via the X-Request-ID header."
)

TOOLS=("Read" "Edit" "Bash" "Write" "Grep" "Glob")
FILES=("src/upload.ts" "src/auth/middleware.ts" "src/db/pool.rs" "src/session/manager.rs" "src/api/handlers.go" "src/lib/retry.ts" "Dockerfile" "src/components/RegisterForm.tsx" "src/store/dashboard.sql" "src/middleware/logging.rs")

# --- Queue dequeue (session start) ---
write_line "{\"type\":\"queue-operation\",\"operation\":\"dequeue\",\"timestamp\":\"$(ts_offset 0)\",\"sessionId\":\"$SESSION_ID\"}"

echo "Wrote: session start"

OFFSET=2
MSG_COUNTER=0

for ((i = 0; i < NUM_EVENTS; i++)); do
  IDX=$((i % ${#PROMPTS[@]}))
  PROMPT="${PROMPTS[$IDX]}"
  RESPONSE="${RESPONSES[$IDX]}"
  TOOL="${TOOLS[$((i % ${#TOOLS[@]}))]}"
  FILE="${FILES[$IDX]}"
  USER_UUID="u-$(printf '%04d' $((i * 3)))"
  ASST_UUID="a-$(printf '%04d' $((i * 3 + 1)))"
  TOOL_UUID="a-$(printf '%04d' $((i * 3 + 2)))"
  TOOL_ID="toolu_$(printf '%06x' $((RANDOM)))"
  TOOL_RESULT_UUID="u-$(printf '%04d' $((i * 3 + 3)))"

  # User prompt
  write_line "{\"parentUuid\":null,\"isSidechain\":false,\"userType\":\"external\",\"cwd\":\"/Users/test/dev/myproject\",\"sessionId\":\"$SESSION_ID\",\"version\":\"2.1.15\",\"gitBranch\":\"feature/test\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"$PROMPT\"},\"uuid\":\"$USER_UUID\",\"timestamp\":\"$(ts_offset $OFFSET)\",\"permissionMode\":\"default\"}"
  OFFSET=$((OFFSET + 3))

  # Assistant response with tool use
  write_line "{\"parentUuid\":\"$USER_UUID\",\"isSidechain\":false,\"userType\":\"external\",\"cwd\":\"/Users/test/dev/myproject\",\"sessionId\":\"$SESSION_ID\",\"version\":\"2.1.15\",\"gitBranch\":\"feature/test\",\"message\":{\"model\":\"claude-opus-4-5-20251101\",\"id\":\"msg_$(printf '%04d' $i)\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"$RESPONSE\"},{\"type\":\"tool_use\",\"id\":\"$TOOL_ID\",\"name\":\"$TOOL\",\"input\":{\"file_path\":\"$FILE\",\"command\":\"echo test\"}}],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":$((500 + RANDOM % 2000)),\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":$((50 + RANDOM % 500)),\"service_tier\":\"standard\"}},\"requestId\":\"req_$(printf '%04d' $i)\",\"type\":\"assistant\",\"uuid\":\"$ASST_UUID\",\"timestamp\":\"$(ts_offset $OFFSET)\"}"
  OFFSET=$((OFFSET + 5))

  # Tool result
  write_line "{\"parentUuid\":\"$ASST_UUID\",\"isSidechain\":false,\"userType\":\"external\",\"cwd\":\"/Users/test/dev/myproject\",\"sessionId\":\"$SESSION_ID\",\"version\":\"2.1.15\",\"gitBranch\":\"feature/test\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"tool_use_id\":\"$TOOL_ID\",\"type\":\"tool_result\",\"content\":\"Tool executed successfully.\",\"is_error\":false}]},\"uuid\":\"$TOOL_RESULT_UUID\",\"timestamp\":\"$(ts_offset $OFFSET)\",\"toolUseResult\":{\"stdout\":\"Tool executed successfully.\",\"stderr\":\"\",\"interrupted\":false,\"isImage\":false},\"sourceToolAssistantUUID\":\"$ASST_UUID\"}"
  OFFSET=$((OFFSET + 2))

  MSG_COUNTER=$((MSG_COUNTER + 1))
  echo "Wrote: prompt/response pair $MSG_COUNTER ($TOOL on $FILE)"
done

# Final assistant wrap-up
FINAL_UUID="a-final"
write_line "{\"parentUuid\":\"u-$(printf '%04d' $((NUM_EVENTS * 3)))\",\"isSidechain\":false,\"userType\":\"external\",\"cwd\":\"/Users/test/dev/myproject\",\"sessionId\":\"$SESSION_ID\",\"version\":\"2.1.15\",\"gitBranch\":\"feature/test\",\"message\":{\"model\":\"claude-opus-4-5-20251101\",\"id\":\"msg_final\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"All done! The changes are ready for review.\"}],\"stop_reason\":\"end_turn\",\"stop_sequence\":null,\"usage\":{\"input_tokens\":3200,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":25,\"service_tier\":\"standard\"}},\"requestId\":\"req_final\",\"type\":\"assistant\",\"uuid\":\"$FINAL_UUID\",\"timestamp\":\"$(ts_offset $OFFSET)\"}"

TOTAL_LINES=$(wc -l < "$SESSION_FILE" | tr -d ' ')
echo ""
echo "Done. $TOTAL_LINES lines written to $SESSION_FILE"
echo ""
echo "To test with the agent:"
echo "  SANNAI_CLAUDE_DIR=$TARGET_DIR cargo run --manifest-path agent/Cargo.toml -- start"
echo ""
echo "Then verify:"
echo "  curl -s http://127.0.0.1:9847/health | jq"
echo "  curl -s http://127.0.0.1:9847/sessions | jq"
echo "  curl -s http://127.0.0.1:9847/sessions/$SESSION_ID | jq"
echo "  curl -s http://127.0.0.1:9847/sessions/$SESSION_ID/events | jq"
