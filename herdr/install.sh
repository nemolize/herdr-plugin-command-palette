#!/bin/sh
# Build hop: fetch the prebuilt binary for this host from the matching GitHub
# Release, so installing needs no toolchain (docs/design.md §11).
#
# Runs as bare `sh` with nothing sourced, so everything it needs is carried here
# rather than borrowed from a shell profile.
#
# Not exercised by `herdr plugin link`, which skips [[build]] entirely (§13).
# This path is only tested by a real `herdr plugin install`.
set -eu

REPO=nemolize/herdr-plugin-command-palette
BIN_NAME=herdr-command-palette
ROOT=${HERDR_PLUGIN_ROOT:-$(pwd)}
DEST_DIR="$ROOT/bin"
DEST="$DEST_DIR/$BIN_NAME"

die() { printf 'command palette: %s\n' "$*" >&2; exit 1; }

# Termux sets TERMUX_VERSION itself, so the name cannot collide (§2). The
# directory check is a safety net, kept because propagation is observed
# behaviour rather than a documented guarantee.
is_termux() {
  [ -n "${TERMUX_VERSION:-}" ] && return 0
  [ -d /data/data/com.termux/files/usr ]
}

# Select by platform, not architecture: `aarch64` alone does not distinguish ARM
# Linux from Termux, and since #3 the two take different assets (§11).
asset_for_host() {
  arch=$(uname -m)
  case $(uname -s) in
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "$BIN_NAME-aarch64-apple-darwin" ;;
        x86_64)        echo "$BIN_NAME-x86_64-apple-darwin" ;;
        *)             return 1 ;;
      esac
      ;;
    Linux)
      if is_termux; then
        case "$arch" in
          aarch64|arm64) echo "$BIN_NAME-aarch64-linux-android" ;;
          *)             return 1 ;;
        esac
      else
        case "$arch" in
          x86_64)        echo "$BIN_NAME-x86_64-unknown-linux-musl" ;;
          aarch64|arm64) echo "$BIN_NAME-aarch64-unknown-linux-musl" ;;
          *)             return 1 ;;
        esac
      fi
      ;;
    *) return 1 ;;
  esac
}

# A binary built for the wrong platform still exits 0 on Termux while silently
# losing DNS and user lookup (§2), so the check has to read what the artefact IS
# rather than whether it runs.
#
# Both readers are optional on a minimal host. When neither is present the
# assertion is skipped rather than failed — refusing to install over a missing
# `file` would be worse than installing unverified.
assert_platform() {
  path=$1
  want=$2

  if command -v file >/dev/null 2>&1; then
    described=$(file -b "$path" 2>/dev/null) || described=""
  else
    described=""
  fi
  [ -n "$described" ] || return 0

  case "$want" in
    *-linux-android)
      # The Android build carries Android's own interpreter, and that is
      # precisely why it works where a static generic-Linux build does not.
      case "$described" in
        *Android*|*"/system/bin/linker64"*) return 0 ;;
        *) die "fetched asset is not an Android binary ($described)" ;;
      esac
      ;;
    *-unknown-linux-musl)
      case "$described" in
        *Android*|*"/system/bin/linker64"*)
          die "fetched an Android binary for a generic Linux host ($described)" ;;
        *ELF*) return 0 ;;
        *) die "fetched asset is not an ELF binary ($described)" ;;
      esac
      ;;
    *-apple-darwin)
      case "$described" in
        *Mach-O*) return 0 ;;
        *) die "fetched asset is not a Mach-O binary ($described)" ;;
      esac
      ;;
  esac
}

fetch() {
  url=$1
  out=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$out" "$url"
  else
    die "neither curl nor wget is available to download the binary"
  fi
}

version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/herdr-plugin.toml" | head -n 1)
[ -n "$version" ] || die "could not read version from herdr-plugin.toml"

asset=$(asset_for_host) || die "no prebuilt binary for $(uname -s) $(uname -m)"
url="https://github.com/$REPO/releases/download/v$version/$asset"

mkdir -p "$DEST_DIR"
tmp="$DEST.download"
trap 'rm -f "$tmp"' EXIT INT TERM

fetch "$url" "$tmp" || die "could not download $url"
[ -s "$tmp" ] || die "downloaded an empty file from $url"

assert_platform "$tmp" "$asset"

chmod +x "$tmp"
mv "$tmp" "$DEST"
trap - EXIT INT TERM

printf 'command palette: installed %s\n' "$asset"
