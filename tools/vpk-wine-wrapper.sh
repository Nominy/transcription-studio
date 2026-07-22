#!/usr/bin/env bash
# Runs the portable, framework-dependent vpk.dll through a genuine Windows
# .NET runtime hosted under Wine, instead of the Linux-native `dotnet`.
#
# Velopack's vpk CLI decides which OS to package for based on which runtime
# build is executing it (a property baked into the runtime binary itself,
# not a --runtime/-r flag): the Linux-native `dotnet` always produces a
# Linux .AppImage regardless of --runtime win-x64. Hosting the identical
# vpk.dll under a Windows-native dotnet.exe -- which Wine lets us execute
# directly -- makes it report as Windows and unlocks the Windows-only
# packaging path (Setup.exe, shortcuts, MSI, signtool). Confirmed by hand:
# `vpk pack --help` under Wine lists --shortcuts/--msi/--signParams, which
# are entirely absent under the Linux-native dotnet.
#
# Absolute Unix path arguments (e.g. --packDir /path/to/stage) are
# translated to their Wine drive-letter equivalent via `winepath -w` so the
# Windows-side vpk sees a path it understands; every other argument passes
# through untouched.
#
# Configure via environment (defaults match the standard per-user layout
# this repo's self-hosted runner setup uses):
#   WINE_DOTNET_EXE  Path to a Windows-native dotnet.exe extracted from the
#                    matching win-x64 runtime zip. Required.
#   VPK_DLL          Path to the portable vpk.dll (tools/net8.0/any/vpk.dll
#                    inside the pinned `vpk` NuGet package). Required.

set -euo pipefail

: "${WINE_DOTNET_EXE:?WINE_DOTNET_EXE must point at a Windows-native dotnet.exe}"
: "${VPK_DLL:?VPK_DLL must point at vpk.dll}"

args=()
for arg in "$@"; do
    if [[ "$arg" == /* ]]; then
        args+=("$(winepath -w "$arg")")
    else
        args+=("$arg")
    fi
done

export WINEDEBUG="${WINEDEBUG:--all}"
exec wine "$WINE_DOTNET_EXE" "$VPK_DLL" "${args[@]}"
