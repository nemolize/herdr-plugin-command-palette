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

# The CLI reports API failures in its JSON body and still exits 0, so the
# response is what has to be read — not $?.
response=$("$HERDR_BIN_PATH" plugin pane open \
  --plugin "$HERDR_PLUGIN_ID" \
  --entrypoint "$ENTRYPOINT" \
  --placement popup \
  --focus 2>&1) || true

error_message=$(printf '%s' "$response" | json_str message)

[ -n "$error_message" ] || exit 0

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

# A popup is up, and this is where §6's toggle was to go. It cannot be built on
# herdr 0.8.2:
#
#   - `plugin.pane.close` and `plugin.pane.focus` both REQUIRE a pane_id.
#   - `plugin.pane.open` returns only {"type":"ok"} — the PluginPaneInfo the
#     schema defines is not what the running server sends, over the CLI or the
#     raw socket.
#   - The plugin pane does not appear in `pane.list`, and the pane process
#     receives no HERDR_PANE_ID of its own.
#
# So our own popup cannot be named, and the only primitive that would close it
# is parameterless `popup.close` — which closes whatever is up regardless of
# owner. §6 rejects that on purpose: it would let a keypress meant for us
# dismiss another plugin's UI.
#
# Reporting is therefore the whole behaviour for now. It is §6's safe direction
# — the worst outcome is one redundant message, where the alternative risks
# closing someone else's window. Dismissal stays Esc, which the palette has.
printf 'command palette: a popup is already open (press Esc in it to close)\n' >&2
exit 1
