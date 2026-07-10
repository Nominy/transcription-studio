#!/usr/bin/env bash
set -euo pipefail

export CARGO_TERM_COLOR=never
export RUST_BACKTRACE=0
export RUST_TEST_THREADS=1

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1; then
  WINDOWS_HOME="${USERPROFILE:-}"
  WINDOWS_HOME="${WINDOWS_HOME//\\//}"
  WINDOWS_USER="${USERNAME:-User}"
  CARGO_BIN=""
  for candidate in \
    "$HOME/.cargo/bin/cargo.exe" \
    "$WINDOWS_HOME/.cargo/bin/cargo.exe" \
    "C:/Users/$WINDOWS_USER/.cargo/bin/cargo.exe" \
    "/c/Users/$WINDOWS_USER/.cargo/bin/cargo.exe" \
    "/mnt/c/Users/$WINDOWS_USER/.cargo/bin/cargo.exe"
  do
    if [[ -f "$candidate" ]]; then
      CARGO_BIN="$candidate"
      break
    fi
  done
  if [[ -z "$CARGO_BIN" ]]; then
    printf 'cargo executable not found\n' >&2
    exit 127
  fi
fi

"$CARGO_BIN" test --locked -p studio-app --bin studio-app \
  app::tests::autoresearch_polish_score -- \
  --exact --nocapture --test-threads=1
