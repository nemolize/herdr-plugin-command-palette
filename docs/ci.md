# CI and static analysis

What runs, what does not, and why. The "why not" is recorded as deliberately as
the "why" — every tool below was considered and the omissions are decisions, not
oversights.

Two facts shape every choice here:

- **The plugin ships prebuilt binaries to strangers.** Users install through
  `herdr plugin install` and never hold a toolchain. Whatever the maintainer's
  CI does not catch, nobody downstream is positioned to catch.
- **`[profile.release]` sets `strip = true`.** A published asset carries no
  symbols, so after the fact there is no way to determine from the artefact
  which dependency versions went into it. `Cargo.lock` at the release tag is the
  only record, which makes the lockfile a distribution manifest rather than a
  build detail.

## What runs

| Tool | Where | Why it is in |
|---|---|---|
| `cargo fmt --all --check` | `Lint` | Zero false positives, about a second, and it keeps diffs reviewable — the scarce resource when an agent writes most of the code and reformats regions it touches. |
| `cargo clippy --locked --all-targets -- -D warnings` | `Lint` | The default lint group is a correctness floor, and the tree already passes at `-D warnings`, so adopting it costs nothing today and catches real bug classes later. |
| `cargo test --locked` | `Test` | The 41 existing tests, which were being run by hand until now. |
| `cargo build --release --locked` for both musl targets | `Build` | `cargo test` compiles the test profile only. This is the sole check that exercises `[profile.release]` (LTO, `opt-level = "z"`, strip) and the per-target `rust-lld` pins in `.cargo/config.toml` — breakage that would otherwise surface for the first time at a release tag. It covers the two release assets a Linux runner can build unaided; the macOS and Android assets need another host or the NDK, so they stay #10's to verify. |
| `cargo deny --locked check` | `Audit` | See the group table below. |

`Lint`, `Test` and `Build` are separate jobs rather than one `just ci` step, so a
red X names which check failed without opening the log, and the three run
concurrently. The shape — job ids as the displayed name, capitalised, on a pinned
`ubuntu-24.04` — follows `nemolize/web-app-template`, which is the reference
layout across these repositories; the language differs, the conventions should
not.

## What does not run

| Tool | Why it is out |
|---|---|
| `cargo audit` | `cargo deny`'s `advisories` group reads the same RUSTSEC database. Running both creates two failure surfaces for one check. |
| `typos` | It needs a domain-term allow-list from day one (`herdr`, `ratatui`, crossterm key names), and the user-visible strings it would guard are ones the maintainer reads on every manual run of the TUI — a faster feedback loop than CI. |
| `clippy::pedantic` | Its findings are largely style preferences that land as `#[allow]` attributes scattered through `src/`, trading CI noise for source noise and training the reflex that later suppresses a real lint. |
| `rust-version` (MSRV) | Users receive a prebuilt binary and never invoke a Rust toolchain, so an MSRV constrains nobody. The build reproducibility it would nominally provide is delivered by `rust-toolchain.toml`, which is enforced rather than declared. |

## cargo-deny, per group

`deny.toml` enforces three groups and reports the fourth.

| Group | Setting | Why |
|---|---|---|
| `advisories` | `yanked = "deny"` | The only check that fires on events outside this repo — an advisory published against an unchanged lockfile. That is what the schedule exists for. |
| `licenses` | explicit allow-list | `Cargo.toml` declares `license = "MIT"`, a claim a copyleft transitive dependency would falsify in a binary that is actually distributed. |
| `sources` | `unknown-registry`/`unknown-git = "deny"` | A mechanical assertion that nothing git- or path-sourced enters a shipped binary. The realistic failure is not malice but an agent adding a `git = "..."` dependency to work around an unreleased upstream fix. |
| `bans` | `multiple-versions = "warn"` | A duplicate version is upstream's resolution, not something the author's diff caused, so failing on it would be red for a reason absent from the change under review. |

The allow-list was built by enumerating every licence expression in the tree, so
it holds no entry that was not needed by some crate; `deny.toml` records why the
non-obvious inclusions and omissions are what they are. Three entries cover
crates the targets built here never reach, which `check licenses` reports as
`license-not-encountered` warnings — a wider list than the resolved graph needs
is the safe direction, and narrowing it would break the day a target that does
reach them is added.

## The blocking model

**Nothing blocks, by design.** There is no branch protection and no required
check, so a red X reports without preventing a merge.

`continue-on-error` appears nowhere and should not be added: it turns the commit
status green, which hides a failure rather than making it advisory.

The intended reading of a red `Lint`, `Test` or `Build` is *the diff broke
something* — each fails only for a reason present in the change. `Audit` is kept
in its own workflow precisely so it cannot dilute that: it is the one job that
can fail for reasons outside the diff, and if a required-check ruleset is ever
added, the three CI jobs can be required and `Audit` left out.

On a scheduled failure `Audit` opens (or comments on) an issue, because a red
cron run on a repo with one maintainer otherwise reaches nobody. That step is not
the durable backstop: GitHub disables a public repository's schedules after 60
days without activity, and a job that never runs cannot report on itself.
Dependabot alerts are enabled on the repository and are what survives that —
for advisories. Nothing mirrors a crate *yank* into an alert, so `yanked = "deny"`
detection stops with the schedule and does not come back on its own.

`.github/dependabot.yml` opens the PRs that act on those alerts, and covers the
one thing nothing else observes: every action is pinned by SHA, and a SHA never
moves on its own. Dependabot rewrites the trailing version comment along with
the SHA, so the pin survives the bump.

## Toolchain parity

`rust-toolchain.toml` pins 1.97.1 and is the single definition — it governs the
maintainer's shell, CI, and any future workflow, so there is one version to bump
rather than one per consumer.

CI installs it via `actions-rust-lang/setup-rust-toolchain` with no `toolchain:`
input, which reads the file. That action's `rustflags` input defaults to
`-D warnings`; it is set to `""` here deliberately. A global `RUSTFLAGS` applies
to dependency compilation — so an upstream deprecation would turn this repo's CI
red on someone else's code — and it is part of cargo's fingerprint, which would
diverge the CI cache from every local build. Clippy's `-D warnings` is passed
per-invocation instead, where it is scoped to this crate.

`justfile` holds the check definitions and each CI job runs one recipe
(`just lint`, `just test`, `just build-musl`, `just deny`), so the commands exist
once rather than as lists kept in sync by discipline. `just ci` runs the three CI
jobs' recipes together, reproducing a CI failure locally with no push — given the
two tools CI pins and installs for itself:

```sh
cargo install just --version 1.58.0 --locked
cargo install cargo-deny --version 0.20.2 --locked
```

Both are installed here at the versions the workflows pin, because a local tool
that disagrees with CI's is the "clean here, red there" divergence this setup
exists to prevent. `brew install just` is fine for everyday use and is what most
setups already have; it just tracks the current formula rather than 1.58.0, so
reach for the pinned install when a CI result and a local one disagree.

Every action is pinned by full commit SHA. A tag is mutable, and a repo that
audits its Rust dependencies should hold its own workflow supply chain to the
same standard.

## Not covered here

**Release supply chain.** Signing, checksums, and build provenance for the
published assets are not addressed by anything in this document. `cargo deny`
guards the *dependency* chain; a compromised dependency ships inside a perfectly
signed asset, and conversely no amount of dependency auditing detects a tampered
release. That work belongs to the release workflow (#10) and is called out here
so a green CI badge is not mistaken for having covered it.

**Termux.** Nothing in CI runs on Android. #10's Android asset is built and
inspected, never executed.
