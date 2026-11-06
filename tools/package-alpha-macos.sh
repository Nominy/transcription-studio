#!/usr/bin/env bash
# Builds Exaudio natively on macOS and packs it with Velopack (channel `alpha`) for the VPS
# alpha feed. Env: VERSION (default: crates/studio-app version; baked into the binary as
# STUDIO_BUILD_VERSION), RUNTIME (osx-arm64 | osx-x64, default: host), STUDIO_ALPHA_FEED_URL
# (alpha feed the build updates from, default: the platform feed), MACOSX_DEPLOYMENT_TARGET
# (default 14.0), optional APPLE_* signing inputs below.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

version="${VERSION:-}"
channel="alpha"
pack_id="Exaudio"
pack_title="Exaudio"
pack_authors="Nominy"
main_exe="Exaudio"
bundle_id="io.github.nominy.exaudio"
stage_dir="${STAGE_DIR:-$repo_root/target/velopack/macos-stage}"
output_dir="${OUTPUT_DIR:-$repo_root/target/velopack/Releases}"
publish_dir="${PUBLISH_DIR:-$repo_root/target/velopack/Publish}"
source_dir="$repo_root/target/release"
runtime="${RUNTIME:-}"
deployment_target="${MACOSX_DEPLOYMENT_TARGET:-14.0}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "package-alpha-macos.sh must run on macOS." >&2
  exit 1
fi

if [[ -z "$version" ]]; then
  version="$({
    awk -F '"' '/^version[[:space:]]*=/ { print $2; exit }' \
      "$repo_root/crates/studio-app/Cargo.toml"
  })"
fi
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "VERSION must be SemVer (major.minor.patch[-prerelease]): '$version'" >&2
  exit 1
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

if ! command -v dotnet >/dev/null 2>&1; then
  echo "The repo-local vpk tool requires .NET SDK 8. Run: dotnet tool restore" >&2
  exit 1
fi

run_vpk() {
  (cd "$repo_root" && dotnet tool run vpk -- "$@")
}

python3 - "$repo_root" "$stage_dir" "$output_dir" "$publish_dir" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()
names = ["STAGE_DIR", "OUTPUT_DIR", "PUBLISH_DIR"]
paths = [pathlib.Path(value).resolve() for value in sys.argv[2:]]
for path in paths:
    try:
        relative = path.relative_to(root)
    except ValueError as error:
        raise SystemExit(f"refusing to operate outside repo root: {path}") from error
    if not relative.parts:
        raise SystemExit(f"refusing to use repo root as a release directory: {path}")
for left_index, left in enumerate(paths):
    for right_index in range(left_index + 1, len(paths)):
        right = paths[right_index]
        if left == right or left in right.parents or right in left.parents:
            raise SystemExit(
                f"{names[left_index]} and {names[right_index]} must not overlap: "
                f"{left} and {right}"
            )
PY

export MACOSX_DEPLOYMENT_TARGET="$deployment_target"
# Baked into the binary: the version it reports and the update channel it follows.
export STUDIO_BUILD_VERSION="$version"
export STUDIO_UPDATE_CHANNEL="alpha"

echo "Packaging Exaudio $version for $runtime."
echo "MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET"
if [[ -n "${STUDIO_ALPHA_FEED_URL:-}" ]]; then
  echo "STUDIO_ALPHA_FEED_URL=$STUDIO_ALPHA_FEED_URL"
fi

# Build the app, relay and isolated compiler together so shared dependency features stay unified.
cargo build -p studio-app -p studio-relay -p exaudio-wasm --release --locked

rm -rf "$stage_dir"
mkdir -p "$stage_dir" "$output_dir"
cp "$source_dir/studio-app" "$stage_dir/$main_exe"
cp "$source_dir/studio-relay" "$stage_dir/studio-relay"
cp "$source_dir/exaudio-wasm-worker" "$stage_dir/exaudio-wasm-worker"
chmod +x "$stage_dir/$main_exe" "$stage_dir/studio-relay" "$stage_dir/exaudio-wasm-worker"
mkdir -p "$stage_dir/assets/demo"
cp "$repo_root/crates/studio-editor/assets/demo/speaker_1.wav" "$stage_dir/assets/demo/"
cp "$repo_root/crates/studio-editor/assets/demo/speaker_2.wav" "$stage_dir/assets/demo/"

for file in README.md LICENSE; do
  if [[ -f "$repo_root/$file" ]]; then
    cp "$repo_root/$file" "$stage_dir/"
  fi
done

pack_args=(
  --packId "$pack_id"
  --packVersion "$version"
  --packDir "$stage_dir"
  --mainExe "$main_exe"
  --channel "$channel"
  --runtime "$runtime"
  --packTitle "$pack_title"
  --packAuthors "$pack_authors"
  --bundleId "$bundle_id"
  --outputDir "$output_dir"
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

run_vpk pack "${pack_args[@]}"

python3 - "$output_dir" "$publish_dir" "$channel" "$version" <<'PY'
import hashlib
import json
import pathlib
import shutil
import sys

output = pathlib.Path(sys.argv[1]).resolve()
publish = pathlib.Path(sys.argv[2]).resolve()
channel = sys.argv[3]
version = sys.argv[4]
asset_index_name = f"assets.{channel}.json"
asset_index_path = output / asset_index_name
if not asset_index_path.is_file():
    raise SystemExit(f"missing Velopack asset index: {asset_index_path}")

with asset_index_path.open("r", encoding="utf-8-sig") as handle:
    assets = json.load(handle)
if not isinstance(assets, list) or not assets:
    raise SystemExit(f"Velopack asset index contains no current assets: {asset_index_path}")

files = []
seen = set()

def add(relative_name, kind):
    if not isinstance(relative_name, str) or not relative_name:
        raise SystemExit("publish manifest contains an empty relative path")
    pure = pathlib.PurePosixPath(relative_name)
    if (
        pure.is_absolute()
        or any(part in ("", ".", "..") for part in pure.parts)
        or "\\" in relative_name
        or "\n" in relative_name
        or "\r" in relative_name
    ):
        raise SystemExit(f"unsafe publish path: {relative_name!r}")
    source = (output / pathlib.Path(*pure.parts)).resolve()
    try:
        source.relative_to(output)
    except ValueError as error:
        raise SystemExit(f"publish path escapes output directory: {relative_name}") from error
    folded = relative_name.casefold()
    if folded in seen:
        raise SystemExit(f"duplicate publish path: {relative_name}")
    if not source.is_file():
        raise SystemExit(f"publish manifest references a missing file: {relative_name}")
    seen.add(folded)
    files.append((relative_name, kind, source))

for asset in assets:
    if not isinstance(asset, dict):
        raise SystemExit("Velopack asset index entry is not an object")
    add(asset.get("RelativeFileName"), "asset")
for metadata_name in (
    asset_index_name,
    f"RELEASES-{channel}",
    f"releases.{channel}.json",
):
    add(metadata_name, "metadata")

if publish.exists():
    shutil.rmtree(publish)
publish.mkdir(parents=True)
manifest_files = []
for relative_name, kind, source in files:
    destination = publish / pathlib.Path(*pathlib.PurePosixPath(relative_name).parts)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    manifest_files.append(
        {
            "RelativeFileName": relative_name,
            "Kind": kind,
            "Size": destination.stat().st_size,
            "SHA256": hashlib.sha256(destination.read_bytes()).hexdigest().upper(),
        }
    )

manifest = {"Version": version, "Channel": channel, "Files": manifest_files}
manifest_path = publish / "publish-manifest.json"
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
publish_bytes = sum(path.stat().st_size for path in publish.rglob("*") if path.is_file())
print(
    f"PublishDir contains {len(manifest_files)} indexed files plus its manifest "
    f"({publish_bytes} bytes): {publish}"
)
PY

echo "Velopack macOS alpha artifacts:"
find "$publish_dir" -maxdepth 1 -type f -print | sort
