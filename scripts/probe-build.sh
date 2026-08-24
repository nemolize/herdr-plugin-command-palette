#!/bin/sh
out="$HOME/.cache/termux-probe-build.txt"
mkdir -p "$(dirname "$out")"
{
  echo "TERMUX_VERSION=${TERMUX_VERSION:-<unset>}"
  if [ -d /data/data/com.termux/files/usr ]; then echo "usr dir: exists"; else echo "usr dir: missing"; fi
  echo "uname -m: $(uname -m)"
  echo "--- env ---"
  env | sort
} > "$out" 2>&1
echo "build step ran; wrote $out"
