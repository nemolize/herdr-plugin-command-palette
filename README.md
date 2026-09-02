# herdr-plugin-command-palette

Fuzzy-search and run Herdr's own commands, plugin actions, and session targets —
from a popup over the session.

```
┌ Command Palette ───────────────────────────┐
│ > split                                    │
│ ▶ Split pane: right                        │
│   Split pane: down                         │
│                                            │
│ 2/24 · esc to close                        │
└────────────────────────────────────────────┘
```

## Why another palette

At least four palette plugins already exist for Herdr. This one covers two gaps
those cannot reach from where they sit:

- **Herdr's own commands are in the list.** `herdr plugin action list` enumerates
  *plugin* actions only — no API enumerates splitting a pane, zooming, moving a
  tab. Every other palette is therefore a launcher for other plugins. This one
  carries a catalog of built-ins (`herdr/catalog.toml`) so both kinds of entry
  appear in the same search.
- **Termux is supported.** Herdr runs on Termux and a phone is exactly where a
  keyboard-driven palette beats hunting for a keybinding, but no other plugin
  ships an Android build. This one does.

## Install

```sh
herdr plugin install nemolize/herdr-plugin-command-palette
```

No toolchain needed: the install step fetches a prebuilt binary for your host
from the matching GitHub Release. Five assets are published — macOS arm64 and
x86_64, Linux arm64 and x86_64, and **Android arm64 for Termux**.

Termux takes a different asset from ARM Linux, and the script picks it by
platform rather than architecture: a generic Linux build runs on Termux but
silently loses DNS and user lookup, so `aarch64` alone is not enough to decide.
If no asset matches your host the install fails loudly rather than registering a
plugin with no working binary.

Requires **herdr 0.8.2 or newer**.

## Bind a key — required, and not otherwise discoverable

A plugin cannot declare its own keybinding; there is no `[[keys]]` table in the
manifest. Add one to your `config.toml`:

```toml
[[keys.command]]
key = "prefix+ctrl+p"
type = "plugin_action"
command = "command-palette.open"
description = "Command palette"
```

Then `herdr server reload-config`, or press `prefix+shift+r`.

**`type = "plugin_action"` does not appear in `herdr --default-config`.** It
exists in the binary and shipped plugins use it, but reading the documented
config will not lead you to it — which is why this section is here rather than
left to inference.

`prefix+ctrl+p` rather than `prefix+p`: the latter is the natural fit but is
commonly taken already (`mr04vv/herdr-pane-navigator` binds it). Any key works.

## Using it

Type to filter. `Enter` runs the selected entry; `↑` / `↓` move the selection.

Entries that need a target you must choose — `Focus tab…`, `Move pane to tab…`,
`Close workspace…` — open a second list of the live tabs, panes or workspaces
when you pick them. `Esc` there backs out to the command list rather than closing
the palette.

**Dismissal is `Esc`, `Ctrl-C`, or picking an entry.** Pressing the palette's own
key again does not close it — on herdr 0.8.2 a plugin cannot name its own popup,
so closing "ours" specifically is not expressible; the alternative would risk
dismissing another plugin's window. Tracked as
[#12](https://github.com/nemolize/herdr-plugin-command-palette/issues/12).
There is no click-outside-to-dismiss either: no mouse events reach a plugin at
all.

Ordering is frecency — entries you run often and recently rise to the top.

## Configuration

Set `$HERDR_PLUGIN_CONFIG_DIR` (Herdr provides it) and drop a `catalog.toml`
there to replace the shipped catalog:

```toml
checked_against = "0.8.2"

[[command]]
id = "pane.split.right"
title = "Split pane: right"
args = ["pane", "split", "--pane", "{pane}", "--direction", "right"]
contexts = ["pane"]
```

Your file **replaces** the shipped one wholesale rather than merging entry by
entry — a merge would leave you unable to *remove* an entry, which is half of
what correcting a catalog means. Start from
[`herdr/catalog.toml`](herdr/catalog.toml) and edit.

- `args` is appended to herdr's binary and run as written.
- `{pane}`, `{tab}`, `{workspace}` resolve from the invocation's context — the
  pane you were in when the palette opened, not the palette's own pane. An entry
  naming an id the invocation lacks is not offered rather than failing when
  picked.
- `resolve` marks an entry that cannot name its target until open time. Its value
  is the list API whose rows become the second-step candidates (`tab list`,
  `pane list`, `workspace list`), and the chosen id is substituted for `{}`.
- `contexts` limits where the entry is offered.

Popup size and placement live in the manifest (`herdr-plugin.toml`), not here —
`herdr plugin pane open` has no `--width` / `--height` flags, so the action hop
cannot pass a size. The shipped values are 60 columns by 45% of the rows, sized
against the **keyboard-up** state: on a phone, raising the keyboard is what using
the palette means, and sizing against the idle state loses about a third of the
box exactly when it matters.

## The catalog drifts, and says so when it does

Nothing enumerates Herdr's built-ins, so the catalog is hand-written argv — and
it goes stale when Herdr changes its CLI. Two things make that visible rather
than silent:

- `checked_against` pins the herdr release the entries were last verified on.
  When the running herdr is older, the palette says so in its footer rather than
  refusing — most entries still work. It is deliberately not
  `min_herdr_version`, which is the manifest's hard install gate.
- A failed dispatch surfaces as an error naming the command id, so a drifted
  entry reports itself the first time you use it.

## Development

`herdr plugin link` installs from a local checkout and **skips `[[build]]`
entirely**, so the install script's fetch path is only exercised by a real
`herdr plugin install`. The everyday loop is:

```sh
cargo build --release
mkdir -p bin && cp target/release/herdr-command-palette bin/
herdr plugin link .
```

`just` runs the checks CI runs. Design rationale — every measurement and every
rejected alternative — is in [`docs/design.md`](docs/design.md).

## License

MIT. See [LICENSE](LICENSE).
