#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
runtime="${RUNTIME:-}"
osxcross_target_dir="${OSXCROSS_TARGET_DIR:-}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "build-alpha-macos-osxcross.sh must run on Linux." >&2
  exit 1
fi

case "$runtime" in
  osx-arm64)
    rust_target="aarch64-apple-darwin"
    osxcross_arch="aarch64"
    cc_alias="oa64-clang"
    cxx_alias="oa64-clang++"
    ;;
  osx-x64)
    rust_target="x86_64-apple-darwin"
    osxcross_arch="x86_64"
    cc_alias="o64-clang"
    cxx_alias="o64-clang++"
    ;;
  *)
    echo "RUNTIME must be set to osx-arm64 or osx-x64 (got '${runtime:-unset}')." >&2
    exit 1
    ;;
esac

if [[ -n "$osxcross_target_dir" ]]; then
  osxcross_bin="$osxcross_target_dir/bin"
  if [[ ! -d "$osxcross_bin" ]]; then
    echo "OSXCROSS_TARGET_DIR does not contain a bin directory: $osxcross_bin" >&2
    exit 1
  fi
  export PATH="$osxcross_bin:$PATH"
fi

for tool in cargo rustup; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Required build tool '$tool' was not found on PATH." >&2
    exit 1
  fi
done

if command -v osxcross-conf >/dev/null 2>&1; then
  eval "$(osxcross-conf)"
fi

cc_path=""
cxx_path=""
ar_path=""
compiler_prefix=""
if [[ -n "${OSXCROSS_TARGET_DIR:-}" && -n "${OSXCROSS_TARGET:-}" ]]; then
  compiler_prefix="$OSXCROSS_TARGET_DIR/bin/$osxcross_arch-apple-$OSXCROSS_TARGET"
  if [[ -x "$compiler_prefix-clang" ]]; then
    cc_path="$compiler_prefix-clang"
  fi
  if [[ -x "$compiler_prefix-clang++" ]]; then
    cxx_path="$compiler_prefix-clang++"
  fi
  if [[ -x "$compiler_prefix-ar" ]]; then
    ar_path="$compiler_prefix-ar"
  else
    for ar_candidate in "$OSXCROSS_TARGET_DIR/bin/"*"-apple-$OSXCROSS_TARGET-ar"; do
      if [[ -x "$ar_candidate" ]]; then
        ar_path="$ar_candidate"
        break
      fi
    done
  fi
fi

if [[ -z "$cc_path" ]] && command -v "$cc_alias" >/dev/null 2>&1; then
  cc_path="$(command -v "$cc_alias")"
fi
if [[ -z "$cxx_path" ]] && command -v "$cxx_alias" >/dev/null 2>&1; then
  cxx_path="$(command -v "$cxx_alias")"
fi

if [[ -z "$cc_path" || -z "$cxx_path" ]]; then
  echo "Missing osxcross compiler wrappers for $runtime. Expected versioned wrappers from osxcross-conf or '$cc_alias' and '$cxx_alias' on PATH." >&2
  echo "Add osxcross/bin to PATH or set OSXCROSS_TARGET_DIR." >&2
  exit 1
fi
if [[ -z "$ar_path" ]]; then
  if [[ -n "${OSXCROSS_TARGET_DIR:-}" && -d "$OSXCROSS_TARGET_DIR/bin" ]]; then
    ar_search_dir="$OSXCROSS_TARGET_DIR/bin"
  else
    ar_search_dir="$(dirname "$cc_path")"
  fi
  for ar_candidate in "$ar_search_dir"/*-apple-*-ar; do
    if [[ -x "$ar_candidate" ]]; then
      ar_path="$ar_candidate"
      break
    fi
  done
fi

if [[ -z "$ar_path" ]]; then
  echo "Missing a versioned osxcross ar wrapper for $runtime in the osxcross bin directory." >&2
  exit 1
fi

if [[ -n "${OSXCROSS_SDK:-}" ]]; then
  export SDKROOT="$OSXCROSS_SDK"
fi

target_env_suffix="${rust_target//-/_}"
target_env_suffix="${target_env_suffix^^}"
cc_target_env="CC_${rust_target//-/_}"
cxx_target_env="CXX_${rust_target//-/_}"
ar_target_env="AR_${rust_target//-/_}"
linker_target_env="CARGO_TARGET_${target_env_suffix}_LINKER"

export "$cc_target_env=$cc_path"
export "$cxx_target_env=$cxx_path"
export "$ar_target_env=$ar_path"
export "$linker_target_env=$cc_path"

rustup target add "$rust_target"

cd "$repo_root"
echo "Cross-compiling Transcription Studio for $runtime ($rust_target)."
echo "$linker_target_env=$cc_path"
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
fi
if [[ -n "${MACOSX_DEPLOYMENT_TARGET:-}" ]]; then
  echo "MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET"
fi

cargo build \
  -p studio-app \
  -p studio-relay \
  --release \
  --locked \
  --target "$rust_target"
