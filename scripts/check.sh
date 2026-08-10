#!/usr/bin/env bash
# End-to-end smoke test: build hello, install into the Emery emulator,
# verify the Rust heartbeat appears in the logs.
#
# Prerequisites: pebble-tool 5.0.x with SDK 4.17, and
# `rustup target add thumbv7m-none-eabi`.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELLO_DIR="$REPO_ROOT/examples/hello"
MARKER="HEARTBEAT"
TIMEOUT_SECS=120   # first emulator boot can be slow

echo "==> cargo checks"
(cd "$REPO_ROOT" && cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite)

echo "==> unit tests"
# Added in Phase 1. These guard logic that regressed more than once: the
# wscript relocation guardrail (which catches -C relocation-model=pic failing
# to reach rustc) and FixedBuf's UTF-8 truncation in the panic handler.
(cd "$REPO_ROOT" && cargo test -p ferrite)
(cd "$REPO_ROOT" && python3 -m unittest discover -s examples/hello/tests)

echo "==> pebble build"
(cd "$HELLO_DIR" && pebble build)

echo "==> install to emery emulator and watch logs (up to ${TIMEOUT_SECS}s)"
LOG_FILE="$(mktemp)"
cleanup() {
    if [[ -n "${INSTALL_PID:-}" ]] && kill -0 "$INSTALL_PID" 2>/dev/null; then
        kill "$INSTALL_PID" 2>/dev/null || true
        wait "$INSTALL_PID" 2>/dev/null || true
    fi
    rm -f "$LOG_FILE"
}
trap cleanup EXIT

(cd "$HELLO_DIR" && pebble install --emulator emery --logs >"$LOG_FILE" 2>&1) &
INSTALL_PID=$!

for _ in $(seq 1 "$TIMEOUT_SECS"); do
    if grep -q "panicked" "$LOG_FILE"; then
        echo "FAIL: app panicked:"
        grep "panicked" "$LOG_FILE"
        exit 1
    fi
    if grep -q "$MARKER" "$LOG_FILE"; then
        echo "PASS: heartbeat observed:"
        grep -m 3 "$MARKER" "$LOG_FILE"
        exit 0
    fi
    if ! kill -0 "$INSTALL_PID" 2>/dev/null; then
        echo "FAIL: pebble install exited early:"
        tail -20 "$LOG_FILE"
        exit 1
    fi
    sleep 1
done

echo "FAIL: no '$MARKER' within ${TIMEOUT_SECS}s. Last log lines:"
tail -20 "$LOG_FILE"
exit 1
