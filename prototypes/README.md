# Prototypes

Two implementations of the same minimal palette, built to decide the
implementation language (#4). Kept as the evidence behind that decision, and —
for whichever language wins — as the starting point for the real thing.

Both do the same work, deliberately: an input line, a fuzzy-filtered candidate
list, keyboard navigation (up/down/enter/esc), and `herdr pane list` shelled out
with its JSON parsed into candidates. Scope-matched so the comparison is about
the languages rather than about two different programs.

## Building

```sh
cd go && go build -ldflags="-s -w" -o palette-go .
cd rs && cargo build --release
```

Neither prints anything useful outside a terminal — the TUI needs a TTY, so
running one from a non-interactive shell gets you the "could not open TTY"
error rather than a palette. Open one through a Herdr plugin pane to see it work.

## Measurements

From macOS arm64, no extra system packages (#4):

| | Go | Rust |
|---|---|---|
| Source lines | 178 | 184 |
| Binary, stripped | 3.74 MB | 0.53 MB |
| Startup, less the 4.9 ms `herdr` baseline | ~26 ms | ~19 ms |
| Builds to green | 2 | 1 |
| Cross-compiles to `aarch64-linux-android` | yes | **no — wants the NDK** |

That last row decided it, because Termux needs an Android-ABI binary and not a
static Linux one (#3). Whether Rust can produce that artefact some other way —
built on the device itself — is open in #5.
