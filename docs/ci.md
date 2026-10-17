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

Each check below runs as a `justfile` recipe, and the first column names it.

`ci.yml` runs these recipes; which job runs which is read from
`.github/workflows/ci.yml`.

| Recipe | Why it is in |
|---|---|
| `just lint` | `cargo fmt --all --check`: zero false positives, about a second, and it keeps diffs reviewable — the scarce resource when an agent writes most of the code and reformats regions it touches. `cargo clippy --locked --all-targets -- -D warnings`: the default lint group is a correctness floor, and the tree already passes at `-D warnings`, so adopting it costs nothing today and catches real bug classes later. |
| `just zizmor` | Audits the workflows themselves: every external `uses:` must be a full commit SHA, its trailing version comment must name that SHA's tag, and the known footguns (persisted checkout credentials, caches feeding a release, template injection) fail the job. Suppressions are inline `# zizmor: ignore[...]` comments on the line they excuse, each with its reason beside it — never on an external `uses:` line, whose comment must be the tag alone. |
| `just zizmor-test` | The repository's own pins all carry their tag, so `just zizmor`'s version-comment check never fires on them, and a check that stopped working — zizmor renaming the audit it filters on, or `ZIZMOR_OFFLINE` set in the shell — would pass the same way. This writes fixture workflows into a temporary directory and asserts that `just zizmor` rejects a pin with no version comment, naming its line, and accepts one whose comment is its tag. |
| `just actionlint-probe` | `release-plan.yml` calls `release.yml` as `./.github/workflows/release.yml` under `# zizmor: ignore[self-repository]`, because the pinned actionlint rejects the `$/` form zizmor asks for. Nothing else here runs actionlint, so nothing would say when a newer one accepts `$/` and the ignore can go. This runs the `actionlint` first on `PATH` on a fixture whose `uses:` is the `$/` form and passes while it is rejected. In CI that is the version `mise.toml` pins; locally, `mise exec -- just actionlint-probe` runs the same one, and a Homebrew build ahead of the pin can answer differently. It fails once the form is accepted, and its message lists the steps to drop the ignore and then the probe itself. It fires when the pin in `mise.toml` is bumped, not before. |
| `just actionlint-probe-test` | The probe passes in today's world, so one that never fails looks the same as one that works. This feeds it a form actionlint accepts, and an actionlint that cannot start, and asserts the removal steps and the "could not tell" failure; it also asserts the verdict for each kind of actionlint output, including the unreadable-callee finding the same rule reports. |
| `just test` | The unit tests, which were being run by hand until now. |
| `just palette-e2e` | The unit tests stop at the seams: `Screen` needs a real terminal, so nothing in-process sees a pick become a running command. This drives the built binary through a PTY against a stubbed herdr, and asserts that a rejected dispatch is readable in the pane rather than printed to a stderr the closing popup takes with it — the shape of "I picked it and nothing happened". |
| `just open-test` | The action hop tells a popup collision from any other open failure by reading herdr's error envelope, whose shape changed between 0.8.2 and 0.9 (docs/design.md §6). A shape it stops matching shows only as a different message, so this runs the hop against a stub answering each envelope and asserts the message and exit status. |
| `just catalog-e2e-test` | `catalog-e2e` fails an entry herdr ran but answered with a response the palette reports as a failure — no envelope, an `error`, a missing or `null` `result` (issue #146). Against a herdr that answers well that check never fires, so a broken one looks the same as a passing one; this feeds it each answer and asserts the verdict. It does the same for the resize and swap effect check, feeding it layouts where the pane did not move or moved the wrong way, and an answer in which herdr names the other action (issue #119). It feeds the verdict checks (`herdr/catalog_verdicts.py`) their answers directly, and resolves entries' placeholders (`herdr/catalog_placeholders.py`) without a herdr, including an entry whose `resolve` it has no target for (issue #157). |
| `just popup-collision-e2e-test` | Against a herdr that collides correctly, `popup-collision-e2e`'s failure branches never fire, so a broken verdict would pass unnoticed. This feeds it each kind of press and asserts the verdict, and checks that each press is read by the run id herdr returns rather than by its place in the log list. |
| `just release-test` | The release publishes from `package.json`'s version while `install.sh` reads `herdr-plugin.toml`'s, and `Cargo.toml`/`Cargo.lock` carry a third copy. `Release`'s `Publish` job asserts only `herdr-plugin.toml`, against the tag, and never reads `Cargo.toml` or `Cargo.lock`, so this, in the required `Test` job, is the only check that sees every copy. It asserts all four agree, and runs the Changesets version step on a fixture repository — bump, one changelog entry per changeset, every manifest synced, changeset consumed — plus the release-planning decisions (`docs/release.md`). |
| `just catalog-e2e` | The catalog hand-writes argv for every entry, and the unit tests check it against a `--help` table transcribed at one herdr version, which goes stale silently — three entries shipped broken that way, the last one valid in flags and arity and wrong only in combination (issue #24). This fetches a real herdr and runs every entry against it, so a constraint herdr adds is caught by the tool that added it rather than by a user whose pick does nothing. Resize and swap entries must also move their pane, since herdr answers both well when they change nothing (issue #119). |
| `just popup-collision-e2e` | `open-test` asserts the hop's verdict on envelopes transcribed from herdr, so it keeps passing when herdr changes the envelope — which is how #99 shipped. This links a fixture plugin whose popup stays open into a real herdr, has herdr run `open.sh` twice as its action, and asserts the first press opens the popup and the second reports the collision (#133). A first press that opens nothing fails as such, since the second would then have nothing to collide with. |
| `just build-musl` | `cargo test` compiles the test profile only. This is the sole check that exercises `[profile.release]` (LTO, `opt-level = "z"`, strip) and the per-target `rust-lld` pins in `.cargo/config.toml` — breakage that would otherwise surface for the first time in the release build itself. It covers the two musl release assets, the ones a Linux runner can build unaided. |

`Lint`, `Test`, `E2ETests` and `Build` are separate jobs rather than one, so a
red X names which check failed without opening the log, and the four run
concurrently. The shape — job ids as the displayed name,
capitalised, on a pinned `ubuntu-24.04` — follows `nemolize/web-app-template`,
which is the reference layout across these repositories; the language differs,
the conventions should not.

`E2ETests` is the one job that fetches a herdr binary; tests that stub herdr or
need none ride `Test` instead. The split is by what a check must install, not
by what kind of test it is.

These recipes have steps in the `Audit` and `Release` workflows, not in `ci.yml`:

| Recipe | Why it is in |
|---|---|
| `just deny` | Runs in its own `Audit` workflow rather than CI. See the group table below. |
| `just build-release <target>` | Builds one release asset. `Release` runs it for each of the five targets, which cuts the release (docs/design.md §11); CI reaches it only through `just build-musl`, for the two musl targets. Adds the three assets CI's `Build` job cannot reach — the two macOS targets need their own runner, Android the NDK — so nothing else compiles those three before the release build. On the Changesets route that is `Release Plan` building the release draft's commit, before any tag exists; on the manual tag route it is the run the tag triggers (`docs/release.md` says which route a release is on). A `workflow_dispatch` dry run builds them earlier on either route. |

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
unapproved (`docs/release.md`) rather than a routine merge route.

`continue-on-error` appears nowhere and should not be added: it turns the commit
status green, which hides a failure rather than making it advisory.

A red `Test` or `Build` is meant to say *the diff broke something*: each fails
for a reason present in the change, which is what makes them safe to require.
No required check is fully insulated from outside this repo, though. Every job
installs tools and fetches dependencies at run time — GitHub for actions and
tool binaries, crates.io, the Rust toolchain host, the npm registry in `Test`,
`herdr.dev` in `E2ETests`; `.github/workflows/ci.yml` shows which job uses
which — so an outage, a GitHub API error or a rate limit can turn any of the
four red against an unchanged diff. A re-run clears that.

Two checks can also go red against an unchanged diff for a reason a re-run does
not clear. `E2ETests` resolves a herdr release rather than pinning one, so a
constraint herdr adds can turn it red against an unchanged catalog. That is the
point of the job — a silent catalog drift is exactly what issue #24 was opened
about — but it means a red `E2ETests` is worth reading before assuming the diff
caused it. `Lint`'s `zizmor` step asks GitHub about every pinned action, so a
published advisory against a pinned action, or a tag moved or deleted from under
its version comment, turns `Lint` red against an unchanged workflow. The author
re-pins that action to a tag that exists and is not flagged; for
`actions/checkout`, `scripts/zizmor.test.mjs` pins it too, in the fixtures
`just zizmor-test` audits, so that pin moves with it. This is accepted rather
than split into a second, non-blocking run: pins are what the check exists to
hold, and a non-blocking run would let a pull request that adds or keeps a
vulnerable pin, or one that no tag names, merge. The cost is that every pull
request waits for the re-pin.

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
`-D warnings`; it is set to `""` here deliberately. The default would turn a
rustc warning into an error in every compile of this crate, including `Build`'s
release builds for the musl targets and `Release`'s for all five. A warning only
a release target or profile raises is one host clippy never compiles, so it
would fail those builds on a commit that passes `Lint`. A global `RUSTFLAGS` is
also part of cargo's fingerprint, so it would diverge the CI cache from every
local build. Clippy's `-D warnings` is passed per-invocation instead, where it
is scoped to this crate.

`justfile` holds the check definitions and each CI job runs them as recipes, so
the commands exist once rather than as lists kept in sync by discipline.
`just ci` runs the recipes of every `ci.yml` job but `E2ETests` (those need a
fetched herdr), reproducing a CI failure locally with no push. Of the five
tools CI pins and installs for itself (`just`, `mise`, `zizmor`, `actionlint`
and `cargo-deny`), `just ci` needs `just`, `zizmor` and `actionlint`, and
`cargo-deny` is for `just deny`. Locally `mise` is what installs `zizmor` and
`actionlint`:

```sh
cargo install just --version 1.58.0 --locked
# mise, at the `version:` of the `jdx/mise-action` step in ci.yml
curl https://mise.run | MISE_VERSION=v<that version> sh
mise --version          # must print that version; a Homebrew mise earlier on PATH wins otherwise
mise install            # zizmor and actionlint, at the versions mise.toml pins
cargo install cargo-deny --version 0.20.2 --locked
```

zizmor's and actionlint's versions live in `mise.toml`. CI's `Lint` job installs
zizmor at the version it reads from there, and actionlint through `jdx/mise-action`
(`taiki-e/install-action` has no actionlint), so a bump is one edit. Without mise,
`cargo install zizmor --locked --version` that version works the same, and
actionlint's release page has binaries for its version.

`just zizmor` also needs `jq` and a logged-in `gh`: outside CI it takes
`gh auth token` for the online audits, and fails rather than running them offline.
`just zizmor-test` needs the same, plus Node.

All five are installed here at the versions CI installs, because a local tool
that disagrees with CI's is the "clean here, red there" divergence this setup
exists to prevent. `brew install just` is fine for everyday use and is what most
setups already have; it just tracks the current formula rather than 1.58.0, so
reach for the pinned install when a CI result and a local one disagree.

`just release-test` also needs Node — `.node-version` names the major CI
installs — and `pnpm install --frozen-lockfile --ignore-scripts` for the Changesets CLI that
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

## Releases

How a release is cut, retried and recovered is in [`docs/release.md`](release.md).

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
