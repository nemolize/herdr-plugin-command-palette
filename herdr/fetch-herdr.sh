#!/bin/sh
# Fetch a herdr binary for this host into $1 (default ./bin), so CI can run the
# catalog E2E against a real herdr (issue #24). herdr.dev/latest.json is what
# `herdr update` itself reads, and it carries a sha256 per asset.
#
# Not the plugin's own install path — that is herdr/install.sh, which fetches
# this plugin's binary. This one fetches herdr.
set -eu

MANIFEST=${HERDR_MANIFEST_URL:-https://herdr.dev/latest.json}
DEST_DIR=${1:-./bin}

die() { printf 'fetch-herdr: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux)  os=linux ;;
  Darwin) os=macos ;;
  *)      die "no herdr asset for $(uname -s)" ;;
esac

case "$(uname -m)" in
  x86_64)        arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *)             die "no herdr asset for $(uname -m)" ;;
esac

key="$os-$arch"

manifest=$(curl -fsSL --retry 3 "$MANIFEST") || die "could not fetch $MANIFEST"

# Capture before splitting: a `read` fed by a command substitution discards the
# helper's exit status, so a failure would surface only as an empty field later.
fields=$(printf '%s' "$manifest" | python3 -c '
import json, sys
d = json.load(sys.stdin)
key = sys.argv[1]
url = d.get("assets", {}).get(key)
sum = d.get("sha256", {}).get(key)
if not url or not sum:
    sys.exit(f"manifest has no url and sha256 for {key}")
print(url, sum, d.get("version", "?"))
' "$key") || die "could not read the $key asset out of $MANIFEST"

read -r url sum version <<EOF
$fields
EOF

[ -n "$url" ] && [ -n "$sum" ] || die "manifest has no $key asset"

mkdir -p "$DEST_DIR"
tmp="$DEST_DIR/herdr.download"
trap 'rm -f "$tmp"' EXIT INT TERM

curl -fsSL --retry 3 -o "$tmp" "$url" || die "could not download $url"

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp" | cut -d' ' -f1)
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$tmp" | cut -d' ' -f1)
else
  die "neither sha256sum nor shasum is available to verify the download"
fi

[ "$actual" = "$sum" ] || die "checksum mismatch for $url (want $sum, got $actual)"

chmod +x "$tmp"
mv "$tmp" "$DEST_DIR/herdr"
trap - EXIT INT TERM

printf 'fetch-herdr: installed herdr %s (%s) to %s/herdr\n' "$version" "$key" "$DEST_DIR"
