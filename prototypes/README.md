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

That last row decided #4 for Go, because Termux needs an Android-ABI binary and
not a static Linux one (#3). **#5 measured it and it does not hold**: Termux is
itself a Bionic environment, so an on-device `cargo build` succeeds unaided
(49.6 s, first attempt, no C toolchain); and `ubuntu-latest` ships the NDK
preinstalled, so CI reaches the target in 22 s. Both artefacts carry
`interpreter /system/bin/linker64`, and DNS and `getpwuid_r` — the pair a static
Linux build loses — both work.

On-device, stripped, both toolchains from `pkg`:

| | Go | Rust |
|---|---|---|
| Binary | 4.98 MB | 0.59 MB |

With the capability gap gone the two are level on anything that matters at this
scale, and the choice went to **Rust** (#4, #5). The 7 ms and the 4.4 MB are
real and neither is perceptible on a keypress.
