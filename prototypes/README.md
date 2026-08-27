# Prototypes

Two implementations of the same minimal palette, built to decide the
implementation language (#4, revisited in #5). Kept as the evidence behind that
decision; `rs/` is the starting point for the real thing.

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

That last row decided #4 for Go. The row is still true as written — **but #5
measured it as a deciding axis and it does not hold there**, because Rust
reaches the target by two routes that do not cross-compile from macOS.

On-device (Termux, both toolchains from `pkg`), stripped:

| | Go | Rust |
|---|---|---|
| Binary | 4.98 MB | 0.59 MB |
| Cold `--release` build | — | 49.6 s, first attempt |

CI (`ubuntu-latest`, NDK preinstalled) built the same target in 22 s. Both Rust
artefacts carry `interpreter /system/bin/linker64`.

**The language is Rust; `docs/design.md` §14.5 is the decision record** and
carries what was and was not verified.
