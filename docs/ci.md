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
| `cargo build --release --locked` for both musl targets | `Build` | `cargo test` compiles the test profile only. This is the sole check that exercises `[profile.release]` (LTO, `opt-level = "z"`, strip) and the per-target `rust-lld` pins in `.cargo/config.toml` — breakage that would otherwise surface for the first time at a release tag. It covers the two release assets a Linux runner can build unaided; the macOS and Android assets need another host or the NDK, so `Release` is the only thing that compiles them. |
| `cargo deny --locked check` | `Audit` | See the group table below. |
| `cargo build --release --locked` for all five targets | `Release` | Cuts the release (docs/design.md §11). Adds the two assets `Build` cannot reach — macOS needs its own runner, Android the NDK — so the first time those two compile is a tag, unless the `workflow_dispatch` dry run is used first. |

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
Renovate is what survives that: it runs on its own cadence rather than this
repo's Actions schedule, so a disabled cron does not take it with it. It covers
the thing nothing else observes — every action is pinned by SHA, and a SHA never
moves on its own; Renovate treats that as a digest update and rewrites the
trailing version comment with it, so the pin survives the bump.

A crate *yank* still has no watcher. Renovate proposes upgrades and GitHub's
advisory alerts fire on vulnerabilities, but neither reports a yank, so
`yanked = "deny"` is only enforced while the `Audit` schedule runs.

`renovate.json` extends `local>nemolize/renovate-config`, the shared preset the
other repositories here use — so cadence, automerge policy and grouping are
settled in one place rather than per repo. Dependabot is deliberately not
configured: two bots proposing updates for the same manifests duplicates every
PR and splits the automerge policy across two config formats.

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

## Cutting a release

Merging the release PR release-please keeps open cuts the release: it writes the
version into `Cargo.toml`, `Cargo.lock` and `herdr-plugin.toml`, then opens a
**draft** GitHub Release on the merge commit. `Release-Please` calls `Release`
directly rather than leaving a tag to trigger it — a tag the default
`GITHUB_TOKEN` writes starts no workflow run, so the five assets would never
build, and `workflow_call` is an invocation rather than an event, which is what
keeps this working without a PAT or a GitHub App.

The draft is what makes the publish all-or-nothing in time as well as in asset
count. A public release with no binaries yet would 404 for anyone installing in
that window, since `install.sh` derives its download URL from the manifest
version, and a failed target would leave that state permanently.
`action-gh-release` keeps an existing release's draft flag while it uploads and
clears it once every asset is attached — so the release goes live, and **the git
tag comes into existence**, only at that point. Nothing in `Release` reads the
tag from git: both jobs check out the release commit by SHA, and the manifest
assertion compares the tag as a string.

The release PR's own checks need one click before they run. A pull request the
default `GITHUB_TOKEN` opens creates its workflow runs in an approval-required
state — the one thing that token can trigger, and only that far — so `Lint`,
`Test` and `Build` sit behind "Approve workflows to run" on every release PR and
on every re-sync as `main` moves. Approving is the release's own review step
rather than an extra one, which is why this is accepted rather than worked
around.

Pushing a tag matching `v[0-9]*.[0-9]*.[0-9]*` by hand still publishes, so a
release can be cut when release-please cannot. That route writes no changelog and
leaves `.release-please-manifest.json` behind, from which the next release PR
computes its version — so a hand-cut release means editing the manifest to match
in the same breath as `herdr-plugin.toml`, which the publish already asserts
against the tag. Either way the same matrix builds the five assets of §11 with
`fail-fast: false`. Three properties are worth stating because each fails
silently otherwise:

- **The publish is all-or-nothing.** `install.sh` requests exactly one asset
  name per platform, so a release carrying four of the five is not a partial
  release — it is one platform whose install fetches a 404 and registers a
  plugin with no binary. `Publish` enumerates all five by name before it runs,
  rather than uploading whatever `dist/` happens to hold.
- **The tag must agree with `herdr-plugin.toml`.** The install script derives
  the download URL from the manifest's version, not from the tag, so a
  disagreement publishes assets nobody will ever ask for. Asserted at publish.
- **Each artefact is checked against the name it is about to be published
  under.** A mispublished asset fails at exec on the user's machine rather than
  at install, and on Termux both wrong picks still exit 0 (§2) — so the check
  reads what the binary *is* via `file`, the same assertion `install.sh` makes
  downstream, at the one point a bad artefact can still be stopped.

The NDK is pinned by writing its version into the path rather than following
`ANDROID_NDK_ROOT`: the runner image carries several and rotates which one that
variable names. The step fails when the pinned path is absent instead of falling
through to whichever NDK is present, since a pin that degrades to "any NDK" is
not a pin.

`workflow_dispatch` builds the matrix against a given ref and publishes nothing.
It exists because macOS and Android compile nowhere else — `Build` covers only
the two musl targets — so without it the first compile of three of the five
assets would be the tag itself. `Publish` runs on a dispatch too, stopping short
of the two steps that need a tag (the manifest assertion and the upload), so its
five-asset check and checksum are rehearsed rather than first executing under a
tag, where a fix would cost a new version.

Registering the dispatch needs the workflow on the default branch, so *this*
workflow could not be rehearsed before it merged. That is a one-time bootstrap
cost, not a standing property: a dispatch runs the selected ref's version of the
file, so a later branch editing `Release` rehearses its own version directly.

## Not covered here

**Release supply chain.** `Release` publishes a `SHA256SUMS` alongside the five
assets, which is the whole of it — **signing and build provenance are not
addressed by anything here**, and a checksum published beside the artefact it
describes attests only that the two were written together. `cargo deny` guards
the *dependency* chain, a different one: a compromised dependency ships inside a
perfectly signed asset, and no amount of dependency auditing detects a tampered
release. Called out so a green CI badge is not mistaken for having covered it.

**Termux.** Nothing in CI runs on Android. `Release`'s Android asset is built
and inspected, never executed — the assertion that it is an Android binary reads
what the artefact *is*, which is the most a Linux runner can say about it. That
it works on a device rests on #5's on-device run of a probe crate sharing the
target triple; a device check after the first release is what closes it.
