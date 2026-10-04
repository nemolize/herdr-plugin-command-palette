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

Each check below runs as a `justfile` recipe; which job runs which recipe is
read from `.github/workflows/`.

| Tool | Why it is in |
|---|---|
| `cargo fmt --all --check` | Zero false positives, about a second, and it keeps diffs reviewable — the scarce resource when an agent writes most of the code and reformats regions it touches. |
| `cargo clippy --locked --all-targets -- -D warnings` | The default lint group is a correctness floor, and the tree already passes at `-D warnings`, so adopting it costs nothing today and catches real bug classes later. |
| `just zizmor` | Audits the workflows themselves: every external `uses:` must be a full commit SHA, its trailing version comment must name that SHA's tag, and the known footguns (persisted checkout credentials, caches feeding a release, template injection) fail the job. Suppressions are inline `# zizmor: ignore[...]` comments on the line they excuse, each with its reason beside it — never on an external `uses:` line, whose comment must be the tag alone. |
| `just zizmor-test` | The repository's own pins all carry their tag, so `just zizmor`'s version-comment check never fires on them, and a check that stopped working — zizmor renaming the audit it filters on, or `ZIZMOR_OFFLINE` set in the shell — would pass the same way. This writes fixture workflows into a temporary directory and asserts that `just zizmor` rejects a pin with no version comment, naming its line, and accepts one whose comment is its tag. |
| `cargo test --locked` | The unit tests, which were being run by hand until now. |
| `python3 herdr/palette-e2e.py` | The unit tests stop at the seams: `Screen` needs a real terminal, so nothing in-process sees a pick become a running command. This drives the built binary through a PTY against a stubbed herdr, and asserts that a rejected dispatch is readable in the pane rather than printed to a stderr the closing popup takes with it — the shape of "I picked it and nothing happened". |
| `python3 herdr/open-test.py` | The action hop tells a popup collision from any other open failure by reading herdr's error envelope, whose shape changed between 0.8.2 and 0.9 (docs/design.md §6). A shape it stops matching shows only as a different message, so this runs the hop against a stub answering each envelope and asserts the message and exit status. |
| `python3 herdr/catalog-e2e-test.py` | `catalog-e2e` fails an entry herdr ran but answered with a response the palette reports as a failure — no envelope, an `error`, a missing or `null` `result` (issue #146). Against a herdr that answers well that check never fires, so a broken one looks the same as a passing one; this feeds it each answer and asserts the verdict. It does the same for the resize and swap effect check, feeding it layouts where the pane did not move or moved the wrong way, and an answer in which herdr names the other action (issue #119). |
| `python3 herdr/popup-collision-e2e-test.py` | Against a herdr that collides correctly, `popup-collision-e2e`'s failure branches never fire, so a broken verdict would pass unnoticed. This feeds it each kind of press and asserts the verdict, and checks that each press is read by the run id herdr returns rather than by its place in the log list. |
| `just release-test` | The release publishes from `package.json`'s version while `install.sh` reads `herdr-plugin.toml`'s, and `Cargo.toml`/`Cargo.lock` carry a third copy; a disagreement surfaces only at publish, where fixing it costs a version. This asserts all four agree, and runs the Changesets version step on a fixture repository — bump, one changelog entry per changeset, every manifest synced, changeset consumed — plus the release-planning decisions ("Cutting a release"). |
| `python3 herdr/catalog-e2e.py` | The catalog hand-writes argv for every entry, and the unit tests check it against a `--help` table transcribed at one herdr version, which goes stale silently — three entries shipped broken that way, the last one valid in flags and arity and wrong only in combination (issue #24). This fetches a real herdr and runs every entry against it, so a constraint herdr adds is caught by the tool that added it rather than by a user whose pick does nothing. Resize and swap entries must also move their pane, since herdr answers both well when they change nothing (issue #119). |
| `python3 herdr/popup-collision-e2e.py` | `open-test` asserts the hop's verdict on envelopes transcribed from herdr, so it keeps passing when herdr changes the envelope — which is how #99 shipped. This links a fixture plugin whose popup stays open into a real herdr, has herdr run `open.sh` twice as its action, and asserts the first press opens the popup and the second reports the collision (#133). A first press that opens nothing fails as such, since the second would then have nothing to collide with. |
| `cargo build --release --locked` for both musl targets | `cargo test` compiles the test profile only. This is the sole check that exercises `[profile.release]` (LTO, `opt-level = "z"`, strip) and the per-target `rust-lld` pins in `.cargo/config.toml` — breakage that would otherwise surface for the first time in the release build itself. It covers the two release assets a Linux runner can build unaided; the macOS and Android assets need another host or the NDK, so `Release` is the only thing that compiles them. |
| `cargo deny --locked check` | Runs in its own `Audit` workflow rather than CI. See the group table below. |
| `cargo build --release --locked` for all five targets | Cuts the release (docs/design.md §11), in the `Release` workflow rather than CI. Adds the three assets CI's `Build` job cannot reach — the two macOS targets need their own runner, Android the NDK — so nothing else compiles those three before the release build. That is `Release Plan` building the release draft's commit, before any tag exists — on the Changesets route, and on the manual tag route too once the tagged version's commit reaches `main`; only a tag pushed before that makes its own run the first. A `workflow_dispatch` dry run builds them earlier on either route. |

`Lint`, `Test`, `E2ETests` and `Build` are separate jobs rather than one, so a
red X names which check failed without opening the log, and the four run
concurrently. The shape — job ids as the displayed name,
capitalised, on a pinned `ubuntu-24.04` — follows `nemolize/web-app-template`,
which is the reference layout across these repositories; the language differs,
the conventions should not.

`E2ETests` is the one job that fetches a herdr binary; tests that stub herdr or
need none ride `Test` instead. The split is by what a check must install, not
by what kind of test it is.

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
| `advisories` | `yanked = "deny"` | The only check that fires on events outside this repo — an advisory published against an unchanged lockfile. That is what the `Audit` schedule exists for. |
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

**`Lint`, `Test`, `E2ETests` and `Build` block; `Audit` reports.** A ruleset on
the default branch requires those four check runs and holds the merge until each
is green. `Audit` is deliberately outside that set, so it reports without
preventing a merge.

The ruleset also requires a pull request, forbids deletion and non-fast-forward
pushes, and asks for no approving review — a repository with one maintainer
would otherwise block on a review nobody can give. It lets the admin role bypass
via pull request, which is the version PR's escape hatch when the check runs sit
unapproved (below) rather than a routine merge route.

`continue-on-error` appears nowhere and should not be added: it turns the commit
status green, which hides a failure rather than making it advisory.

The intended reading of a red `Test` or `Build` is *the diff broke
something* — each fails only for a reason present in the change, which is what
makes them safe to require. Two required checks hold a weaker version of that
property, because each watches something that changes outside this repo.
`E2ETests` resolves a herdr release rather than pinning one, so a constraint
herdr adds can turn it red against an unchanged catalog. That is the point of
the job — a silent catalog drift is exactly what issue #24 was opened about —
but it means a red `E2ETests` is worth reading before assuming the diff caused
it. `Lint`'s `zizmor` step asks GitHub about every pinned action, so a published
advisory, an upstream tag moved or deleted from under its version comment, or a
GitHub API error or rate limit turns `Lint` red against an unchanged workflow.
That is accepted rather than split into a second, non-blocking run: an advisory
or a moved tag wants the same bump Renovate would propose, and an API error
clears on a re-run. `Audit` stays outside the required set, so an advisory
published against an unchanged lockfile reports rather than blocks a merge.

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
`-D warnings`; it is set to `""` here deliberately. A global `RUSTFLAGS` is part
of cargo's fingerprint, so it would diverge the CI cache from every local
build. Clippy's `-D warnings` is passed per-invocation instead, where it is
scoped to this crate.

`justfile` holds the check definitions and each CI job runs them as recipes, so
the commands exist once rather than as lists kept in sync by discipline.
`just ci` runs the recipes of every `ci.yml` job but `E2ETests` (those need a
fetched herdr), reproducing a CI failure locally with no push. Of the three
tools CI pins and installs for itself, `just ci` needs `just` and `zizmor`, and
`cargo-deny` is for `just deny`:

```sh
cargo install just --version 1.58.0 --locked
mise install            # zizmor, at the version mise.toml pins
cargo install cargo-deny --version 0.20.2 --locked
```

zizmor's version lives in `mise.toml`, and CI's `Lint` job installs the version
it reads from there, so a bump is one edit. Without mise,
`cargo install zizmor --locked --version` that version works the same.

`just zizmor` also needs `jq` and a logged-in `gh`: outside CI it takes
`gh auth token` for the online audits, and fails rather than running them offline.
`just zizmor-test` needs the same, plus Node.

All three are installed here at the versions CI installs, because a local tool
that disagrees with CI's is the "clean here, red there" divergence this setup
exists to prevent. `brew install just` is fine for everyday use and is what most
setups already have; it just tracks the current formula rather than 1.58.0, so
reach for the pinned install when a CI result and a local one disagree.

`just release-test` also needs Node — `.node-version` names the major CI
installs — and `pnpm install --ignore-scripts` for the Changesets CLI that
`pnpm-lock.yaml` pins (`packageManager` in `package.json` names the pnpm
version). pnpm rather than npm because `pnpm-lock.yaml` records no version for
the root package, so a release bump has no lockfile copy to leave behind (#95).

Every action is pinned by full commit SHA. A tag is mutable, and a repo that
audits its Rust dependencies should hold its own workflow supply chain to the
same standard. `just zizmor` enforces it, so a floating tag, a branch
reference, or a version comment that is missing or names the wrong release fails
`Lint`. Renovate moves the pins (above); a pin changed by hand takes both the
SHA of the release's tag and that tag as its comment, and `just zizmor` confirms
they agree.

## Cutting a release

Why Changesets, and why no PAT or GitHub App is involved, is recorded in
`docs/adr/0002-changesets-release-planning.md` and
`docs/adr/0001-release-automation.md` along with what was rejected — read them
before proposing a credential or a relaxed ruleset here.

Release intent is written down, not inferred. A pull request with a
user-visible change adds a file under `.changeset/` naming the bump and the
changelog line (`.changeset/README.md` says when one is needed and how to add
it). Nothing reads commit messages or tags to decide the next version, so a
merge commit repeating its PR's subject cannot add a second changelog entry
(#41), and a release still being built cannot make its own changes look
unreleased (#43).

`Release Plan` runs on every push to `main`:

1. **`Version-PR`** — while changesets are pending, `changesets/action` opens or
   updates the single `Version Packages` pull request (branch
   `changeset-release/main`). Its commit runs `pnpm run version-packages`:
   `changeset version` consumes the pending files, bumps `package.json` and
   writes `CHANGELOG.md`, then `scripts/sync-version.mjs` copies the version
   into `Cargo.toml`, `Cargo.lock` and `herdr-plugin.toml`. With nothing
   pending the job does nothing.
2. **`Draft-Release`** — on every push, pending changesets or not: they change
   only the version PR's branch, never the version on `main`.
   `scripts/release-plan.mjs` compares `package.json`'s version with the
   repository's releases, drafts included: a published release means nothing
   to do; no release means draft one at the commit that set the version — the
   version PR's own commit, not whatever `main` has moved on to — with that
   version's `CHANGELOG.md` section as its notes; an existing draft means an
   earlier build failed, so rebuild the commit the draft targets. A tag with no
   release, or a tag beside a draft, stops the run rather than publishing onto
   whatever commit the tag names.
3. **`Assets`** — calls `Release` with the draft's commit and tag. A tag the
   default `GITHUB_TOKEN` writes starts no workflow run, so the five assets
   would never build if a tag were left to trigger them; `workflow_call` is an
   invocation rather than an event, which is what keeps this working without a
   PAT or a GitHub App.

`package.json` exists only for this: it is `private`, publishes nothing to npm,
and holds the version Changesets bumps, which is the official route for
non-npm packages. `just release-test` asserts every manifest agrees with it and
runs the whole version step on a throwaway fixture repository; it rides the
required `Test` job, so a drifted version blocks the merge that caused it.

The draft is what makes the publish all-or-nothing in time as well as in asset
count. A public release with no binaries yet would 404 for anyone installing in
that window, since `install.sh` derives its download URL from the manifest
version, and a failed target would leave that state permanently.
`action-gh-release` keeps an existing release's draft flag while it uploads and
clears it once every asset is attached — so the release goes live, and **the git
tag comes into existence**, only at that point. Nothing in `Release` reads the
tag from git: both jobs check out the release commit by SHA, and the manifest
assertion compares the tag as a string.

**Retrying a failed release.** A failed target leaves the draft unpublished
and untagged, so nothing is public. Either re-run the failed jobs of that
`Release Plan` run, or run `Release Plan` by hand (`gh workflow run
release-plan.yml`) — any later push to `main` does the same. Each rebuilds the
commit the draft targets under the same version; no new changeset or version
bump is involved. A failure the draft's commit will always hit (a code or
build-config fault) cannot be retried away: ship the fix with a changeset as
the next version, and restate the stuck version's changelog entries in that
changeset's note — its own section reaches no published release. Until the
fix's version PR merges, each run rebuilds the broken draft and fails again;
afterwards nothing targets the old version, so delete its draft
(`gh release delete <tag>`) and it stays unreleased. An in-progress
run is never cancelled, so a push landing mid-build waits and then sees the
published release or the draft to retry.

None of this starts without a repository setting no file here can carry:
**Settings → Actions → General → "Allow GitHub Actions to create and approve
pull requests"**. With it off, `changesets/action` cannot open the version PR
at all, and nothing in the workflow itself points at the cause. It is off by
default, so it is the one prerequisite a fresh clone of this configuration does
not inherit.

The version PR's own checks need one click before they run. A pull request the
default `GITHUB_TOKEN` opens creates its workflow runs in an approval-required
state — the one thing that token can trigger, and only that far — so `Lint`,
`Test`, `E2ETests` and `Build` sit behind "Approve workflows to run" on every
version PR and on every update as changesets land. Approving is the release's
own review step rather than an extra one, which is why this is accepted rather
than worked around.

Pushing a tag matching `v[0-9]*.[0-9]*.[0-9]*` by hand still publishes, so a
release can be cut when `Release Plan` cannot. That route writes no changelog,
and the version it tags must already be in `package.json` and every manifest
`just release-test` checks — the publish asserts `herdr-plugin.toml` against the
tag. Either way the same matrix builds the five assets of §11 with
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
It exists because macOS and Android compile nowhere else — CI's `Build` job covers
only the two musl targets — so without it the first compile of three of the five
assets would be the release build: `Release Plan`'s build of the draft's commit,
which on the manual tag route too comes first once the tagged version's commit
reaches `main` — the tag's own run is first only when the tag is pushed before
that. `Publish` runs on a dispatch too, so its five-asset check and checksum are
rehearsed rather than first executing in a release build, where a fix would cost
a new version. It skips five steps there: the manifest assertion and the upload,
which run only when publishing, and the draft-target assertion with the two steps
preparing it (checking out the release tooling, setting up Node), which run only
when `Release Plan` passes a draft's tag. No dry run exercises that assertion's
wiring, then; `just release-test` covers only the script it runs.

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
