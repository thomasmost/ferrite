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

echo "==> cargo fmt"
(cd "$REPO_ROOT" && cargo fmt --all --check)

echo "==> unit tests"
# Added in Phase 1. These guard logic that regressed more than once: the
# wscript relocation guardrail (which catches -C relocation-model=pic failing
# to reach rustc) and FixedBuf's UTF-8 truncation in the panic handler.
(cd "$REPO_ROOT" && cargo test -p ferrite)
(cd "$REPO_ROOT" && python3 -m unittest discover -s examples/hello/tests)

echo "==> pebble build"
# A stale bundle must never be installable: pebble install just reads
# build/<name>.pbw off disk, so if the build failed and we kept going, an old
# binary would pass the heartbeat check and mask the failure.
rm -f "$HELLO_DIR"/build/*.pbw

# Timeout after 5 minutes in case the build hangs. Build failures are FATAL --
# never swallow the exit status.
cd "$HELLO_DIR"
if command -v timeout >/dev/null 2>&1; then
    timeout 300 pebble build
else
    # macOS has no coreutils timeout by default: bash-native watchdog.
    # No subshell around the command -- $! must be pebble itself, or the
    # watchdog would kill a wrapper and orphan the real build.
    pebble build &
    BUILD_PID=$!
    for _ in $(seq 1 300); do
        kill -0 "$BUILD_PID" 2>/dev/null || break
        sleep 1
    done
    if kill -0 "$BUILD_PID" 2>/dev/null; then
        kill "$BUILD_PID" 2>/dev/null || true
        wait "$BUILD_PID" 2>/dev/null || true
        echo "FAIL: pebble build timed out after 300s"
        exit 1
    fi
    # Propagate the build's real exit status (set -e aborts here on failure).
    wait "$BUILD_PID"
fi
cd "$REPO_ROOT"

# Belt and braces: the build must have produced a fresh bundle.
if [[ ! -f "$HELLO_DIR/build/hello.pbw" ]]; then
    echo "FAIL: pebble build produced no bundle"
    exit 1
fi

echo "==> install to emery emulator and watch logs (up to ${TIMEOUT_SECS}s)"
LOG_FILE="$(mktemp)"
cleanup() {
    if [[ -n "${INSTALL_PID:-}" ]] && kill -0 "$INSTALL_PID" 2>/dev/null; then
        kill "$INSTALL_PID" 2>/dev/null || true
        wait "$INSTALL_PID" 2>/dev/null || true
    fi
    # Kill any lingering pebble processes from the installation.
    pebble kill --force >/dev/null 2>&1 || true
    rm -f "$LOG_FILE"
}
trap cleanup EXIT

# Drop the subshell so $! gets pebble's PID, not the subshell's.
cd "$HELLO_DIR"
pebble install --emulator emery --logs >"$LOG_FILE" 2>&1 &
INSTALL_PID=$!
cd "$REPO_ROOT"

for _ in $(seq 1 "$TIMEOUT_SECS"); do
    if grep -q "panicked" "$LOG_FILE"; then
        echo "FAIL: app panicked:"
        grep "panicked" "$LOG_FILE"
        exit 1
    fi
    # A heartbeat line only counts when COMPLETE: the trailing u64= field
    # proves the heap_free number is fully flushed, so a partially-written
    # line can neither fail the stability check nor silently shrink the
    # sample. Heartbeat 1 is excluded as warm-up (a first SDK log call could
    # legitimately allocate persistently); leak detection compares 2-4.
    complete='heap_free=[0-9]+ u64=[0-9]+'
    if [[ $(grep -cE "$complete" "$LOG_FILE") -ge 4 ]]; then
        vals=$( (grep -oE "$complete" "$LOG_FILE" | sed -n '2,4p' \
                 | sed -E 's/heap_free=([0-9]+).*/\1/' | sort -u | wc -l) || true )
        if [[ "$vals" -eq 1 ]]; then
            echo "PASS: heartbeat observed with stable heap_free:"
            grep -m 4 "$MARKER" "$LOG_FILE"
            exit 0
        else
            echo "FAIL: heap_free is not stable across heartbeats 2-4 — allocator leak"
            grep "$MARKER" "$LOG_FILE" | head -4
            exit 1
        fi
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
