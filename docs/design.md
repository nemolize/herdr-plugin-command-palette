# Command Palette for Herdr — Design

A Herdr plugin that opens a fuzzy-searchable palette over the session and runs
Herdr operations from it.

Status: design settled. The implementation language is **Rust + ratatui**,
decided by prototype (#4) and confirmed against a measured build route (#5);
platform behaviour, popup sizing, and collision handling were closed against a
live Termux device (#1, #2, #3). The catalog's first twenty entries are chosen
and checked against the CLI (§4). Nothing is left open before coding.

Measured figures throughout come from **one device in one configuration**. The
shape of each conclusion is what generalises — the numbers are not a range to
size against.

## 1. Why another palette

The palette niche is the most contested in the Herdr ecosystem — at least four
plugins already occupy it (`JanTvrdik/herdr-command-palette`,
`ramarivera/herdr-palette`, `thanhdat77/herdr-navigator`,
`mr04vv/herdr-pane-navigator`). A fifth is only worth building where those four
structurally cannot reach.

Two such gaps, both verified against the running binary (herdr 0.8.2):

**Built-in Herdr commands are unreachable through the plugin API.**
`herdr plugin action list` enumerates *plugin* actions only. No API enumerates
Herdr's own operations — splitting a pane, zooming, applying a layout, moving a
tab. Every existing palette is therefore a launcher for other plugins, not for
Herdr itself. Closing that gap means carrying the command catalog inside this
plugin (§4), which is a maintenance cost the others avoided by not trying.

**No plugin declares Termux support.** Every installed manifest reads
`platforms = ["linux", "macos"]` or adds `"windows"`. Herdr runs on Termux, and
a phone is exactly where a keyboard-driven palette beats hunting for a
keybinding — but the ecosystem has no entry there.

Ranking is the third axis: frecency ordering over a catalog that spans both
built-ins and plugin actions is only meaningful once that catalog exists, so it
follows the first gap rather than standing alone.

## 2. Platform reality — what the manifest can and cannot say

`PluginPlatform` is an enum of exactly `linux`, `macos`, `windows`. There is no
`android` and no `termux` value, so **Termux cannot be declared**; it has to
travel inside `linux`. The manifest reads:

```toml
platforms = ["linux", "macos"]
```

Consequences, in the order they bite:

**Herdr cannot route a Termux-specific build.** `[[build]]` accepts a
`platforms` filter, but its values are the same three — so `linux` covers both
glibc desktops and Termux, and one build entry serves both. The host check must
happen *inside* the build script, not in the manifest.

**A stock Linux binary does not run on Termux.** Termux links against bionic,
not glibc; a `aarch64-unknown-linux-gnu` binary fails with a `no such file or
directory` that names an ELF which plainly exists (the missing file is the
interpreter, `/lib/ld-linux-aarch64.so.1`, which Android does not have).
Verified on a live device with a musl build as the control (#1) — so a `grun`
wrapper, and the `glibc-repo` + `glibc-runner` prerequisite it would have
forced on users, is not needed.

### Running is not the bar — the Termux build must target Android, not generic Linux

A second device run (#3) found a failure that a run/fail check does not catch.
Two Go artefacts were built and **both ran**, exit 0, doing real filesystem work.
But they do not behave the same:

| | static generic-Linux build | Android-ABI build |
|---|---|---|
| DNS lookup | **fails** | ok |
| `user.Current()` | **fails** | ok |
| `exec` via `PATH`, temp dir | ok | ok |

The mechanism is visible in the binaries. Termux has no `/etc/resolv.conf` — its
copy lives at `$PREFIX/etc/resolv.conf` — and a generic-Linux build keeps
upstream's hardcoded path, finds nothing, and falls back to `[::1]:53` where
nothing listens. The Android target is patched to the Termux path. `user.Current()`
fails for the same class of reason: no `/etc/passwd`, and no Bionic NSS to ask.

**This inverts the intuitive answer.** "Static, no interpreter" reads as the safer
artefact and is the one that quietly loses the network. The Android build carries
`interpreter /system/bin/linker64` — the same *shape* that killed the glibc build
above — and works precisely because that interpreter is the one Android has.

Two consequences that outlive the language choice:

- **Termux and generic `aarch64` Linux cannot share one binary.** The release
  workflow ships both, and the install script selects **by platform, not by
  architecture alone**.
- **A smoke test must assert the platform, not the exit code.** Both artefacts
  exit 0 on the device even when DNS is broken, so an exit-code check passes the
  artefact that is wrong.

**Host detection is the build script's job**, and the test is
`TERMUX_VERSION`:

```sh
is_termux() {
  [ -n "${TERMUX_VERSION:-}" ] && return 0
  [ -d /data/data/com.termux/files/usr ]
}
```

Termux sets `TERMUX_VERSION` itself, so the name cannot collide with anything
else and the check is unambiguous. Two alternatives were rejected: `$PREFIX` is
a generic name other environments also set, and `uname -o` is a GNU extension
absent from POSIX.

**Verified on a live device (#1):** `TERMUX_VERSION` reaches both the
`[[actions]]` and `[[build]]` processes intact — the build step inherits the full
Termux environment (`PREFIX`, `HOME`, `LD_PRELOAD` all come through), not just
the one variable. On Android the Herdr server can only be started from a Termux
shell, so there is no launch path for the environment to be lost through.

The directory fallback therefore stays a safety net rather than becoming the
primary check. It is kept because propagation is *observed* behaviour, not a
documented guarantee, and the path is Termux's own install prefix.

Note this is a re-implementation, not a reuse: an install script runs as bare
`sh` with nothing sourced, so the three-line predicate is carried by the plugin
rather than borrowed from a shell profile.

**No runtime dependencies.** The existing shell palettes need `fzf` and `jq`,
both extra installs on Termux. Drawing our own TUI makes the plugin a single
self-contained binary on every platform — which is why the language has to be a
compiled one.

**Rust + ratatui**, settled by prototype (#4) and by measuring the build route
that decision turned on (#5); §14.5 is the decision record. Both languages
reach the Android-ABI binary Termux requires from a stock CI runner, so the
axis that first picked Go is void and nothing else was load-bearing.

**On Termux the binary must target `aarch64-linux-android`, not a static
generic-Linux build** — a platform property that holds whichever language wins
(§2). Cross-compiling to it from a non-Android host needs the Android NDK,
which `ubuntu-latest` ships preinstalled; §12 states what that costs the
release workflow.

## 3. Execution model — the TTY constraint shapes everything

Herdr runs `[[actions]]` **headless, with no terminal attached**. An interactive
picker started from an action has nowhere to draw and hangs forever. This is not
a guess: `pane-navigator` documents it in its own source, and `reviewr` is built
the same way.

So the plugin is two hops, and the split is forced:

```
keybinding  →  [[actions]] open   (no TTY; opens the pane, exits)
                     ↓  herdr plugin pane open --placement popup
               [[panes]] palette  (real TTY; renders the TUI, runs the pick)
```

The action does one thing and returns. All rendering, input, and dispatch live
in the pane entrypoint.

`placement` accepts `overlay | popup | split | tab | zoomed`, but for this plugin
the choice is forced rather than preferred: **`overlay` rejects `width` and
`height` outright** (`invalid_params: width and height are only supported when
placement is popup`) and always fills the tab. A palette that sizes itself must
use **`popup`**, which is also the right shape on its own terms — session-modal,
floating over the current view, leaving the tiled layout alone.

Two traps in the CLI surface, both found while building this (#9). **`herdr
plugin pane open --help` lists only `overlay, split, tab, zoomed`** — `popup` is
absent from the help while being accepted by the parser and present in the API
schema's `PluginPanePlacement`. And that subcommand has **no `--width` /
`--height` flags at all**, though the API takes both. So the size lives in the
manifest's `[[panes]]` entry and the action hop cannot pass one — which is
where §5's numbers have to go.

Two constraints measured on a live device (#2), both worth knowing before
writing the pane logic:

- **Only one popup can exist at a time** — a second `plugin.pane.open` fails with
  `popup already open`. What the palette does then is §6.
- **Percentages resolve against the tab, not the pane.** Splitting a tab does not
  shrink the denominator, so a popup always covers its sibling panes.

Dispatch back into Herdr goes through `$HERDR_BIN_PATH` (the full CLI, ~91
methods, JSON responses). The raw newline-delimited JSON socket at
`$HERDR_SOCKET_PATH` is available but unnecessary — the CLI covers the surface
and costs one process spawn per action.

## 4. The command catalog — the part that carries maintenance risk

Because no API enumerates built-in operations, the catalog is **data inside this
plugin**, kept as a TOML file next to the binary rather than compiled in, so a
user can correct an entry without a rebuild.

**The first cut is roughly twenty entries** — the operations reached for daily:
splitting, moving between, zooming and closing panes; creating and switching
tabs; moving between workspaces. The full CLI is ~91 methods, and listing all of
them would make the palette worse, not better: every rarely-used entry is noise
in front of the handful that matter. The catalog grows on demand, from use.

Twenty entries also keeps the drift surface small, which matters because nothing
detects drift automatically (below).

Each entry names a title, the argv to run, and the contexts it is valid in:

```toml
[[command]]
id = "pane.split.right"
title = "Split pane: right"
args = ["pane", "split", "--direction", "right"]
contexts = ["pane"]
```

The risk is honest and worth stating plainly: **this catalog drifts when Herdr
changes its CLI.** Nothing detects the drift automatically. Two mitigations,
neither complete:

- Pin `min_herdr_version` and bump it deliberately when the catalog is
  re-checked against a new release.
- Have the palette surface a failed dispatch as a visible error naming the
  command id, so a drifted entry reports itself the first time it is used
  instead of silently doing nothing.

Entries requiring an argument the user must choose resolve their candidates at
open time from `herdr workspace list` / `herdr tab list` / `herdr pane list`,
which do have APIs. All three return `label` and `focused` alongside the id, so
a candidate row has something readable to show. Such an entry carries a
`resolve` key naming that list, and a `{}` in its `args` where the chosen id is
substituted:

```toml
[[command]]
id = "tab.focus"
title = "Switch to tab…"
args = ["tab", "focus", "{}"]
resolve = "tab list"
contexts = ["global", "workspace", "tab", "pane"]
```

An entry with no `resolve` key runs exactly as written.

A second, simpler substitution covers the ids the invocation already knows:
`{pane}`, `{tab}` and `{workspace}` resolve from `HERDR_PLUGIN_CONTEXT_JSON`
(§8). That is how `pane.close` names the current pane without a picker. An entry
naming an id the invocation lacks is not offered at all, rather than failing when
it is picked.

Reading the CLI to fill the catalog corrected two assumptions this section had
made about the entries, both of which change what can be offered:

**Which operations are dynamic is decided by the CLI's own inconsistency, not by
whether the argument is conceptually a choice.** `split`, `zoom`, `focus`,
`resize` and `swap` all accept `--current`; `pane close`, `tab close`,
`tab focus`, `workspace close` and `workspace focus` take a positional id and
have no `--current`. So the *close* family is dynamic — which the earlier guess
("a target workspace, a layout name") did not anticipate. Closing the current
pane is the exception that escapes it: `$HERDR_PANE_ID` is injected into every
plugin process (§8), so it resolves from the environment rather than a picker.

**`--direction` is not one vocabulary.** `pane split` accepts `right` and `down`
only, while `pane focus`, `swap` and `resize` take all four. A "split left" entry
cannot exist, so the catalog offers two split directions against four focus
directions rather than making the two families look alike.

Plugin actions from `herdr plugin action list` merge into the same list, so one
search covers both built-ins and other plugins.

## 5. Popup dimensions — size against the keyboard-up state

Measured on a Termux device (#2), including a re-measurement that reversed the
first result. **Only the direction is portable.** Absolute figures do not survive
even within one device: the software keyboard's height is a user setting, and the
terminal font size moves the cell grid directly. Both are ordinary
customisations, so the numbers below are one sample of one configuration, never a
range to size against.

```toml
[popup]
width      = 60      # fixed cells
height     = "45%"   # percentage, sized against the keyboard-up state
min_width  = 36
min_height = 8
```

The two axes take different forms, and for opposite reasons.

**Width is fixed cells because columns never move.** Across every sample in every
keyboard state the width held at 41 columns. Columns are set by the environment,
so a percentage silently shrinks the one axis with a hard readability floor — at
`50%` the popup came back 25 columns wide and command titles were already
truncating. The floor is **~36 columns**; below it a smaller box does not help,
only different rendering would.

**Height is a percentage because rows are elastic within a single environment** —
they swung roughly 1.5× on one device without anything being reconfigured. Fixed
cells would overflow the contracted state, which is the inverse of why width
takes them.

**But the percentage is sized against the keyboard-up state, not the idle one.**
Rows *contract* when the software keyboard is raised (43 → 27–29 on the measured
device), and raising it is what using the palette means: typing a filter requires
the keyboard. The idle state is the one the palette is never used in, so sizing
against it — as the first draft's `60%` did — picks the roomy state and then
loses about a third of it exactly when it matters.

`min_height` exists because a percentage of an already-contracted grid can round
down to something useless. A palette showing one candidate is not a smaller
palette; it is a broken one.

### Why the first measurement came out backwards

Worth recording, because the trap generalises to anything measured interactively
on a phone. **Answering a question about the current size requires raising the
keyboard**, so a figure captured at the moment a reply arrives is a keyboard-up
figure regardless of which state it was meant to record. The measurement could
only be taken in one of the two states it was trying to distinguish.

Continuous sampling from inside the pane (`stty size` once a second) broke the
coupling — the log advances whether or not anyone is interacting with it.

The font-shrink explanation offered for the original result is dead either way:
columns never moved in any state, and a font change would move both axes.

### The sidebar is unreachable

A plugin pane cannot draw over the workspace/agent sidebar. On the measured
device the terminal is 82 columns, the sidebar takes 26, and the pane region is
the remaining 56 — and *every* placement clamps to it:

| attempt | inner |
|---|---|
| `popup` `100%` × `100%` | 51 × 53 |
| `popup` `82` × `54` (full physical width) | 51 × 53 — clamped |
| `overlay` | 51 × 53 |
| `zoomed` | 51 × 53 |

Requesting the full physical width explicitly still clamps. Two consequences:

- `width = 60` is not a deliberate margin — it clamps to 56 and means "as wide as
  available" on this device.
- `min_width` is measured against the **pane region**, not the terminal. The
  sidebar has already taken its ~26 columns before the palette sees anything, so
  a narrower device runs out sooner than its terminal width suggests.

Borders cost 2 cells on each axis in both forms. Fixed cells land at exactly the
requested size minus the border; percentages land 1–2 cells under a naive
calculation.

### No compact mode in the first release

It would only earn its place below ~36 columns, and the measured device never
gets there. Revisit if a report arrives from a genuinely narrower terminal.

### Resize is safe

A live resize mid-render (43 ↔ 27 rows, raising and lowering the keyboard) redrew
cleanly — no tearing, corruption, or hang. The TUI needs no special handling for
it beyond respecting `min_height`.

## 6. Popup collision — re-press toggles, a stranger's popup is left alone

Herdr allows one popup per session, so pressing the palette key while a popup is
up gets `popup already open`. Two different situations hide behind that one
error, and they want opposite responses:

| What is open | Response |
|---|---|
| Our own palette | Close it — the keypress reads as a toggle |
| Another plugin's popup | Report it, change nothing |

### The API forbids both, on 0.8.2 — the toggle is not buildable

This section was written from the API schema and is **wrong about what the
running server does**. Building the plugin (#9) established the following
against herdr 0.8.2, over both the CLI and the raw socket:

- `plugin.pane.open` returns `{"type":"ok"}` — **not** the `plugin_pane_opened`
  response carrying `PluginPaneInfo` that the schema defines. No pane id comes
  back.
- `plugin.pane.close` and `plugin.pane.focus` both take a **required** `pane_id`.
- The plugin pane does not appear in `pane.list`.
- The pane process receives no `HERDR_PANE_ID` of its own (§8).

So our own popup cannot be named through any route, and the toggle below cannot
be built. The mechanism the schema implies is real; the server on this version
does not implement its half.

Identifying someone else's popup is not possible. `PluginPaneInfo` is returned
**only** by `plugin_pane_opened`; the pane listings return `PaneInfo`, which
carries no `plugin_id`. So a palette can recognise its own popup (because it
saved the id) but cannot ask what any other popup belongs to.

`popup.close` does exist and takes **no parameters** — it closes whatever popup is
up, regardless of owner. That makes it exactly the wrong primitive here: it
cannot distinguish the two rows of the table above, and using it would let the
palette dismiss another plugin's UI as a side effect of a keypress meant for us.
**Not used.**

### Behaviour

On `popup already open`: report it and exit without touching anything.

That is case 2 of the table, applied to both rows — not because a stranger's
popup and ours deserve the same response, but because on 0.8.2 they cannot be
told apart. It is the safe direction the table already argued for: the worst
outcome is one redundant message, where the alternative risks closing someone
else's window.

**Toggling is deferred, not dropped.** It needs `plugin.pane.open` to return the
pane id it already declares in the schema. When a herdr release ships that, the
id is recorded in `$HERDR_PLUGIN_STATE_DIR` (it must survive between the two
separate processes an invocation spans, §3), and case 1 becomes buildable
exactly as first written.

Note what this costs: the palette's binding opens but does not close it, so
dismissal is `Esc` alone. The claim below that "the palette's own binding
already toggles it" does not hold on this version.

The collision is matched on the error **message**, not the code: the code is the
generic `plugin_pane_open_failed`, which also covers failures that must not be
read as a collision.

### Click-outside-to-dismiss is not available

A GUI modal closes when you click outside it. The palette cannot do that, and the
reason is worth recording so it is not re-investigated later.

**No mouse events reach a plugin.** Nothing in the API delivers clicks to a
plugin process. The two click-shaped fields that exist are unrelated:
`PaneRightClickTarget` configures who handles a right-click (Herdr or the pane),
and `clicked_url` is for link handlers.

**Focus loss is not deliverable either.** `events.subscribe` accepts
`pane.focused` in its subscription vocabulary, but the events actually delivered
are only these three:

```
pane.output_matched | pane.agent_status_changed | pane.scroll_changed
```

`SubscriptionEventKind` does not include `pane.focused`, so subscribing to it
yields nothing. `events.wait` does support `pane_focused`, but it waits for a
**named** pane to gain focus — and the pane the user clicks cannot be named in
advance.

**Polling could approximate it, and is not being used.** `pane.current` and
`pane.list` both expose a `focused` boolean, so the palette could watch its own
pane and exit when focus leaves.

Not worth a timer: `Esc` closes the palette, and that is the dismissal a
keyboard-driven tool is reached for with anyway. On the platform this plugin
targets there is no mouse to click outside with, and the palette's own binding
already toggles it (above).

Dismissal is `Esc`, the toggle key, or picking an entry.

## 7. Ranking

Ordering is frecency — frequency combined with recency, so a command used often
and recently sorts above one used often but long ago. State is a small JSON file
in `$HERDR_PLUGIN_STATE_DIR` (Herdr injects the path), recording per-command hit
counts and last-use timestamps.

Two rules keep it predictable rather than merely clever:

- A typed query filters first; frecency only orders what survives the filter. A
  frequently-used command never outranks a better textual match.
- An empty query shows the frecency list, which is what makes the palette fast
  for the handful of commands any given user actually repeats.

## 8. Environment the plugin receives

Herdr injects, for every plugin process: `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`,
`HERDR_ENV=1`, `HERDR_PLUGIN_ID`, `HERDR_PLUGIN_ROOT`,
`HERDR_PLUGIN_CONFIG_DIR`, `HERDR_PLUGIN_STATE_DIR`,
`HERDR_PLUGIN_CONTEXT_JSON`; and conditionally `HERDR_WORKSPACE_ID`,
`HERDR_TAB_ID`, `HERDR_PANE_ID`, `HERDR_PLUGIN_ACTION_ID`.

**The conditional ids are not among what a plugin *pane* receives.** Dumping the
pane process's own environment (#9) returned exactly nine variables — the
unconditional list above plus `HERDR_PLUGIN_ENTRYPOINT_ID` — and no
`HERDR_PANE_ID`, `HERDR_TAB_ID` or `HERDR_WORKSPACE_ID`. Those reach the
*action* hop, which is a different process.

So `HERDR_PLUGIN_CONTEXT_JSON` is the pane's only route to the ids, and it
carries them under different names:

```json
{"workspace_id": "w46", "tab_id": "w46:t1", "focused_pane_id": "w46:p1",
 "workspace_label": "wevox-mono-web", "invocation_source": "api"}
```

`focused_pane_id` is the pane the user was in when they opened the palette,
which is the one an entry should act on — the palette's own pane is not in it.
This is what the catalog's `{pane}` / `{tab}` / `{workspace}` placeholders
resolve against (§4), and what decides which `contexts` the palette is running
under.

The pane's working directory is **not** the plugin root, so the binary is
launched by absolute path under `$HERDR_PLUGIN_ROOT` — the same reason `reviewr`
does it that way.

## 9. Manifest

`herdr-plugin.toml`, at the **repo root** — not under `herdr/`, which holds the
scripts it points at. `herdr plugin link` reports `plugin_manifest_not_found`
for any other location; `reviewr` is laid out the same way.

```toml
id = "nemolize.command-palette"
name = "Command Palette"
version = "0.1.0"
min_herdr_version = "0.8.0"
platforms = ["linux", "macos"]
description = "Fuzzy-search and run Herdr's own commands, plugin actions, and session targets."

# Fetches the prebuilt binary for this host from the matching GitHub Release.
# Selects the Android-ABI build on Termux, the ordinary Linux one elsewhere —
# the manifest cannot express that split, so the script detects the host itself
# (see docs/design.md §2).
# Skipped by `herdr plugin link`; build locally with cargo when developing.
[[build]]
command = ["sh", "herdr/install.sh"]

# popup is the only placement that accepts width/height (docs/design.md §5).
[[panes]]
id = "palette"
title = "Command Palette"
placement = "popup"
width = 60
height = "45%"
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/herdr-command-palette\""]

[[actions]]
id = "open"
title = "Command palette"
contexts = ["global", "workspace", "tab", "pane"]
command = ["sh", "herdr/open.sh"]
```

## 10. Keybinding — the user must add it by hand

A plugin cannot declare its own keybinding; there is no `[[keys]]` table in the
manifest. The user edits `config.toml`:

```toml
[[keys.command]]
key = "prefix+p"
type = "plugin_action"
command = "nemolize.command-palette.open"
description = "Command palette"
```

then runs `herdr server reload-config` (or `prefix+shift+r`).

Note that `type = "plugin_action"` is **absent from `herdr --default-config`** —
it exists in the binary and shipped plugins use it, but a user reading the
documented config will not find it. The README has to spell this out, because a
palette nobody can bind is a palette nobody uses.

A related gap: **no API reports the user's keybindings**, so the palette cannot
display "this command is bound to prefix+s" beside an entry. Doing so would mean
parsing `config.toml` directly, which is out of scope for the first version.

## 11. Distribution

Prebuilt binaries are published to GitHub Releases and fetched by `[[build]]`,
so **installing needs no toolchain on the user's machine** — the same approach
`reviewr` takes. `herdr plugin install` runs the script, which detects the host
(§2) and pulls the matching asset.

CI builds **five** assets:

| Asset | Host |
|---|---|
| macOS arm64 | macOS (Apple silicon) |
| macOS x86_64 | macOS (Intel) |
| Linux x86_64 | Linux |
| Linux arm64 | ARM Linux |
| **Android arm64** | **Termux** |

Termux is a separate asset from ARM Linux, not a shared one. An earlier draft had
them sharing a static build; a device run (#3) showed that artefact silently
loses DNS and user lookup on Termux (§2). Termux takes
`aarch64-linux-android`; other Linux hosts take the ordinary Linux triple.

Two requirements on the install script, both from that finding:

- **Select by platform, not by architecture.** `aarch64` alone does not
  distinguish ARM Linux from Termux, and the two now take different assets.
- **Fail loudly when no asset matches**, and assert the platform rather than
  trusting the binary to complain. Both Termux artefacts exit 0 even when the
  wrong one is installed, so an exit-code smoke test passes the broken pick. A
  silent failure here leaves the plugin registered with no working binary, and
  the only symptom is a keybinding that does nothing.

## 12. Project shape

Published as open source, so a stranger can `herdr plugin install` it. That
sets a few obligations the design has to carry rather than bolt on afterwards:

- The README documents the keybinding by hand, including that
  `type = "plugin_action"` is absent from `herdr --default-config` (§10). Without
  it the plugin cannot be launched at all.
- Configuration is keyed to no one's machine: popup dimensions, placement, and
  the catalog are all user-overridable through `$HERDR_PLUGIN_CONFIG_DIR`.
- A LICENSE, and CI that both tests and cuts releases for the five assets (§11).
- **Dependencies are pinned**, not floated: `ratatui` 0.30, `crossterm` 0.29,
  with `Cargo.lock` committed. A TUI library's API is exactly the surface an
  agent writes against from recall, and 0.x crates break it on minor bumps; the
  prototype's first build was green against these versions and that is the
  contract to hold.
- **The release workflow needs the Android NDK for the Termux asset.** It is
  preinstalled on `ubuntu-latest` (`ANDROID_NDK_ROOT`), so the cost is a
  `rustup target add aarch64-linux-android` and pointing
  `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` at the NDK's
  `aarch64-linux-android24-clang` — measured at 22 s for a cold build (#5). The
  `24` is the minimum Android API level the binary targets, and matches what #5
  built and ran against on the device. No NDK installation step is required,
  but **pin the NDK by writing its version into the path**
  (`$ANDROID_SDK_ROOT/ndk/<version>`) rather than following `ANDROID_NDK_ROOT`:
  the runner image carries several versions and rotates which one that variable
  points at, so an unpinned workflow changes toolchain under you.

## 13. Development loop

`herdr plugin link` installs from a local checkout and **does not run
`[[build]]`** — only `herdr plugin install` does. So the everyday loop is
`cargo build` → `herdr plugin link .` → invoke the action → iterate, with the
binary placed by hand where the manifest expects it.

The consequence is worth stating separately: **the install script's build path
cannot be exercised through `link`.** Testing it requires a real
`herdr plugin install` from GitHub, which is how the Termux build-step
verification in #1 had to be run. A build script that works locally has not been
tested at all until it has been installed once.

`herdr plugin log` shows what a failed action printed, which is the only way to
see stderr from the headless action process — and the way to confirm whether a
build entry ran.

## 14. Open questions

1. ~~**Termux verification** (#1, #3)~~ — **settled**, see §2. `TERMUX_VERSION`
   propagates to the build step, and the Termux asset must target Android rather
   than generic Linux: a static generic-Linux build runs but silently loses DNS
   and user lookup.
2. ~~**Popup sizing on a narrow screen** (#2)~~ — **settled**, see §5. Width is
   fixed cells, height a percentage sized against the keyboard-up state, both
   with floors. The first measurement had its keyboard labels reversed; the
   re-measurement corrected the direction and the numbers were re-derived from
   it.
3. ~~**Catalog contents**~~ — **settled**, see §4 and `herdr/catalog.toml`.
   Twenty entries, every flag checked against `herdr 0.8.2`'s own help rather
   than recalled. Reading the CLI corrected two assumptions §4 had made about
   what the entries would look like.
4. ~~**Popup collision**~~ — **settled**, see §6. Re-pressing toggles our own
   popup via the `pane_id` returned at open; another plugin's popup is reported
   and left alone, because no API attributes an existing popup to its owner.
5. ~~**Implementation language**~~ — **settled: Rust + ratatui**, by building a
   prototype in each (#4) and then testing the one axis that decision rested on
   (#5). #4 picked Go because Rust could not cross-compile the
   `aarch64-linux-android` binary from macOS without the NDK (`rust-lld: unable
   to find library -ldl` / `-llog` / `-lc`). #5 measured two routes that clear
   it: Termux is itself a Bionic environment, so an on-device `cargo build`
   succeeds unaided (49.6 s, first attempt); and `ubuntu-latest` ships the NDK
   preinstalled, so CI reaches the target in 22 s with a `rustup target add`
   and a linker variable. Both artefacts carry the right interpreter
   (`/system/bin/linker64`). DNS and `getpwuid_r` — the two things a static
   generic-Linux build loses (§2) — were exercised by a probe crate built the
   same way on-device, calling `getpwuid_r` directly rather than through
   `std::env::home_dir`, which would have read `$HOME` and proved nothing.
   Neither prototype binary nor the CI artefact was itself run for those two:
   the CI artefact was inspected, not executed, so its runtime behaviour is
   inferred from the shared target triple. Running as a plugin pane entrypoint
   is likewise still unverified — no herdr server was up.

   With that axis void the two are level on anything load-bearing: Rust is
   smaller (0.59 MB vs 4.98 MB, both measured on-device) and started ~7 ms
   faster in #4's macOS run, which was not re-measured here. Neither margin is
   perceptible on a keypress. The deciding input was preference, admissible as
   a tiebreak once the scores are level — it did not overrule evidence, which
   is what #5 existed to establish.
