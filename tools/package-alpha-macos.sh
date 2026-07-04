#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

version="${VERSION:-}"
channel="${CHANNEL:-alpha}"
pack_id="${PACK_ID:-TranscriptionStudio}"
pack_title="${PACK_TITLE:-Transcription Studio}"
pack_authors="${PACK_AUTHORS:-Nominy}"
bundle_id="${BUNDLE_ID:-ovh.reviewgen.transcription-studio}"
stage_dir="${STAGE_DIR:-$repo_root/target/velopack/macos-stage}"
bundle_dir="${BUNDLE_DIR:-$repo_root/target/velopack/macos-bundle}"
output_dir="${OUTPUT_DIR:-$repo_root/target/velopack/Releases}"
skip_build="${SKIP_BUILD:-0}"
install_vpk="${INSTALL_VPK:-0}"
runtime="${RUNTIME:-}"
deployment_target="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
default_update_feed_url="${STUDIO_DEFAULT_UPDATE_FEED_URL:-}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "package-alpha-macos.sh must run on macOS." >&2
  exit 1
fi

if [[ -z "$version" ]]; then
  version="$(
    awk -F '"' '/^version[[:space:]]*=/ { print $2; exit }' \
      "$repo_root/crates/studio-app/Cargo.toml"
  )"
fi

if [[ -z "$runtime" ]]; then
  case "$(uname -m)" in
    arm64) runtime="osx-arm64" ;;
    x86_64) runtime="osx-x64" ;;
    *)
      echo "Could not infer Velopack runtime from uname -m; set RUNTIME=osx-arm64 or osx-x64." >&2
      exit 1
      ;;
  esac
fi

case "$(uname -m)" in
  arm64) host_runtime="osx-arm64" ;;
  x86_64) host_runtime="osx-x64" ;;
  *) host_runtime="" ;;
esac
if [[ -n "$host_runtime" && "$runtime" != "$host_runtime" ]]; then
  echo "Requested RUNTIME=$runtime, but this host builds $host_runtime. The script does not cross-compile macOS releases." >&2
  exit 1
fi

if [[ "$install_vpk" == "1" || "$install_vpk" == "true" || ! -x "$(command -v vpk || true)" ]]; then
  if ! command -v dotnet >/dev/null 2>&1; then
    echo "vpk is missing and dotnet is not available. Install .NET SDK 8, then run: dotnet tool update -g vpk" >&2
    exit 1
  fi
  dotnet tool update -g vpk
  export PATH="$HOME/.dotnet/tools:$PATH"
fi

if ! command -v vpk >/dev/null 2>&1; then
  echo "vpk is missing. Set INSTALL_VPK=1 or install it with: dotnet tool update -g vpk" >&2
  exit 1
fi

export MACOSX_DEPLOYMENT_TARGET="$deployment_target"
if [[ -n "$default_update_feed_url" ]]; then
  export STUDIO_DEFAULT_UPDATE_FEED_URL="$default_update_feed_url"
fi

echo "Packaging Transcription Studio $version for $runtime."
echo "MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET"
if [[ -n "${STUDIO_DEFAULT_UPDATE_FEED_URL:-}" ]]; then
  echo "STUDIO_DEFAULT_UPDATE_FEED_URL=$STUDIO_DEFAULT_UPDATE_FEED_URL"
fi

if [[ "$skip_build" != "1" && "$skip_build" != "true" ]]; then
  cargo build -p studio-app --release
fi

rm -rf "$stage_dir" "$bundle_dir"
mkdir -p "$stage_dir" "$bundle_dir" "$output_dir"
cp "$repo_root/target/release/studio-app" "$stage_dir/TranscriptionStudio"
chmod +x "$stage_dir/TranscriptionStudio"

for file in README.md Cargo.lock; do
  if [[ -f "$repo_root/$file" ]]; then
    cp "$repo_root/$file" "$stage_dir/"
  fi
done

common_args=(
  --packId "$pack_id"
  --packVersion "$version"
  --packDir "$stage_dir"
  --mainExe "TranscriptionStudio"
  --channel "$channel"
  --runtime "$runtime"
  --packTitle "$pack_title"
  --packAuthors "$pack_authors"
  --bundleId "$bundle_id"
)

vpk bundle \
  --outputDir "$bundle_dir" \
  "${common_args[@]}"

pack_args=(
  --outputDir "$output_dir"
  "${common_args[@]}"
)

# Optional signing inputs:
# APPLE_DEVELOPER_ID_APPLICATION="Developer ID Application: Example, Inc. (TEAMID)"
# APPLE_DEVELOPER_ID_INSTALLER="Developer ID Installer: Example, Inc. (TEAMID)"
# APPLE_NOTARY_PROFILE="notarytool-keychain-profile"
if [[ -n "${APPLE_DEVELOPER_ID_APPLICATION:-}" ]]; then
  pack_args+=(--signAppIdentity "$APPLE_DEVELOPER_ID_APPLICATION")
fi
if [[ -n "${APPLE_DEVELOPER_ID_INSTALLER:-}" ]]; then
  pack_args+=(--signInstallIdentity "$APPLE_DEVELOPER_ID_INSTALLER")
else
  pack_args+=(--noInst)
fi
if [[ -n "${APPLE_CODESIGN_ENTITLEMENTS:-}" ]]; then
  pack_args+=(--signEntitlements "$APPLE_CODESIGN_ENTITLEMENTS")
fi
if [[ -n "${APPLE_NOTARY_PROFILE:-}" ]]; then
  pack_args+=(--notaryProfile "$APPLE_NOTARY_PROFILE")
fi
if [[ -n "${APPLE_KEYCHAIN:-}" ]]; then
  pack_args+=(--keychain "$APPLE_KEYCHAIN")
fi

vpk pack "${pack_args[@]}"

echo "Velopack macOS alpha artifacts:"
find "$output_dir" -maxdepth 1 -type f -print | sort
