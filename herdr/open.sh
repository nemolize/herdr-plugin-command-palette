#!/bin/sh
# Action hop (docs/design.md §3). Runs headless with no TTY, so it opens the
# pane and exits — all rendering lives in the pane entrypoint.
#
# It also meets popup collision (§6): Herdr allows one popup per session, so a
# second press arrives as `popup already open`.
set -eu

: "${HERDR_BIN_PATH:?not running under herdr}"
: "${HERDR_PLUGIN_ID:?not running under herdr}"

ENTRYPOINT=palette

# Reads a top-level "..." string value for a key. The responses this script
# handles are single-line JSON with no nested key of the same name, which is the
# whole reason a small dependency-free reader is enough here.
json_str() {
  sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
}

# The CLI reports an API failure in its JSON body — `{"error":{"message":...}}`
# — so the body is what distinguishes a collision from any other failure. The
# exit status alone cannot: it is 1 for an API error and 1 for a missing binary
# too, and only the body says which.
# `--placement` is deliberately not passed: the manifest's [[panes]] entry
# already declares `popup`, and it governs when the flag is omitted (verified —
# a second open still collides with `popup already open`). Omitting it keeps the
# runtime path off `popup`, the one placement value the CLI accepts but does not
# document, so a future release tightening its parser cannot break the hop.
response=$("$HERDR_BIN_PATH" plugin pane open \
  --plugin "$HERDR_PLUGIN_ID" \
  --entrypoint "$ENTRYPOINT" \
  --focus 2>&1) && status=0 || status=$?

error_message=$(printf '%s' "$response" | json_str message)

# Success is asserted positively, never inferred from the absence of an error
# message. A missing binary, a clap usage error from a future release, or any
# output that is not JSON all produce no `message` — and treating those as
# success is the §11 failure exactly: the plugin stays registered and the
# keybinding silently does nothing.
if [ -z "$error_message" ]; then
  case "$status:$response" in
    0:*'"result"'*) exit 0 ;;
  esac
  printf 'command palette: could not open the palette (exit %s): %s\n' \
    "$status" "${response:-no output}" >&2
  exit 1
fi

# The collision is named in the message, not the code — the code is the generic
# `plugin_pane_open_failed`, which also covers failures that must not be read as
# a collision.
case "$error_message" in
  *"popup already open"*) ;;
  *)
    printf 'command palette: %s\n' "$error_message" >&2
    exit 1
    ;;
esac

# Report only, never close: the popup may be another plugin's (docs/design.md §6).
printf 'command palette: a popup is already open (press Esc in it to close)\n' >&2
exit 1
