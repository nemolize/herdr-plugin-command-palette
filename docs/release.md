# Cutting a release

How a release is cut and recovered. What CI runs, and why, is in
[`docs/ci.md`](ci.md).

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

`Release Plan` runs on every push to `main` — the Changesets route:

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

**Retrying a failed release.** A run that fails while building a target leaves
the draft unpublished and untagged, so nothing is public. A run that fails in
`Publish` can differ: `action-gh-release` keeps calling the API after it
publishes the draft, so the run can end red with the release already live. Run
`gh release view <tag>` first. A release that is not a draft is live: the run
did its work, so retry nothing and delete nothing, and a later `Release Plan`
run skips it. Otherwise, either re-run the failed jobs of that `Release Plan`
run, which works for a day after the build only, or run `Release Plan` by hand
(`gh workflow run release-plan.yml`) — any later push to `main` does the same.
Each rebuilds the commit the draft targets under the same version; no new
changeset or version bump is involved.
Only the checkout of the draft's tree is pinned to that commit: `release.yml`
is called with `./`, which GitHub loads from the run's own commit, and so are
the scripts it checks out into `.release-tooling`. So a fault in either is
fixed by merging the fix to `main` without a changeset, and that push rebuilds
the draft with it, provided the fix needs nothing new from the draft's tree;
re-running the failed jobs reuses the old workflow and cannot pick the fix up.
A fault in anything the release jobs read from the draft's tree — the
`justfile` and toolchain file as much as source and Cargo files — is not fixed
by a rebuild, which reads the same tree. The usual route is to ship the fix
with a changeset as the next version, and restate the stuck version's
changelog entries in that changeset's note — its own section reaches no
published release. Until the fix's version PR merges, each run rebuilds the
broken draft and fails again; afterwards nothing targets the old version.

The stuck version itself can sometimes be released without a new version, by
hand-pushing its tag on the draft's commit (`gh release view <tag> --json
targetCommitish`) with the fix cherry-picked onto it — never on `main`'s head,
which also carries every change merged since the version commit. Nothing here
rehearses that route and no CI job runs on that commit, so treat it as a
fallback; the tag's own steps are in "A hand-pushed tag whose run has not ended
green". Otherwise a new version is required.

Check the draft with `gh release view <tag>`: a draft can be deleted
(`gh release delete <tag>`), after which it stays unreleased until a tag
publishes it, but a release that
is no longer a draft is live, and deleting it would withdraw it. An in-progress
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

The manual tag route — pushing a tag matching `v[0-9]*.[0-9]*.[0-9]*` by hand —
still publishes, so a release can be cut when `Release Plan` cannot. It writes
no changelog, and the version it tags must already be in `package.json` and
every manifest `just release-test` checks — the publish asserts
`herdr-plugin.toml` against the tag. Either way the same matrix builds the five
assets of `docs/design.md` §11 with `fail-fast: false`. Three properties are
worth stating because each fails silently otherwise:

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
  at install, and on Termux both wrong picks still exit 0 (`docs/design.md`
  §2) — so the check reads what the binary *is* via `file`, the same assertion
  `install.sh` makes downstream, at the one point a bad artefact can still be
  stopped.

The NDK is pinned by writing its version into the path rather than following
`ANDROID_NDK_ROOT`: the runner image carries several and rotates which one that
variable names. The step fails when the pinned path is absent instead of falling
through to whichever NDK is present, since a pin that degrades to "any NDK" is
not a pin.

`workflow_dispatch` builds the matrix against a given ref and publishes nothing.
It exists because macOS and Android compile nowhere else — CI's `Build` job
covers only the two musl targets — so without it the first compile of three of
the five assets would be the release build, on either route. `Publish` runs on a
dispatch too, so its five-asset check and checksum are rehearsed rather than
first executing in a release build. It skips the tag-gated steps there: the
manifest assertion and the upload, which run only when publishing, and the
draft-target assertion with the two steps preparing it (checking out the
release tooling, setting up Node), which run only when `Release Plan` passes a
draft's tag. No dry run exercises that assertion, then.

Registering the dispatch needs the workflow on the default branch, so *this*
workflow could not be rehearsed before it merged. That is a one-time bootstrap
cost, not a standing property: a dispatch runs the selected ref's version of the
file, so a later branch editing `Release` rehearses its own version directly.

**A hand-pushed tag whose run has not ended green.** The tag exists from the
push, but its release only once the run's `Publish` job has attached every
asset: the release starts as a draft, so a run failing after that usually leaves
a draft carrying the tag, and rarely a live release (see "The run failed").
`Release Plan` reads a bare tag or a draft as unfinished. Once `main`'s
`package.json` carries the tag's version, each of its runs stops at
`Draft-Release`, before building anything, with "tag … exists without a
release" — or "draft … and a tag … both exist" when a draft is there.
`Version-PR` is unaffected. A red run stays red; a later run (a re-run,
`gh workflow run release-plan.yml`, or a push to `main`) skips once the tag's
release is published.

- **The run is in flight:** wait, and leave the tag alone until the run has
  ended. When it publishes, the release exists and `Release Plan` skips.
- **The run failed:** usually nothing is published, but not always.
  `action-gh-release` keeps calling the API after it publishes the draft, so a
  run can end red with the release already live. Run `gh release view <tag>`
  before touching anything: a draft is unpublished, a release that is not a
  draft is live, and "release not found" means nothing was published.
  - A published release: the run did its work. Delete nothing; a later
    `Release Plan` run skips it.
  - A transient fault with no published release (a runner or network flake):
    re-run the failed jobs of the tag's run. That reuses the tag's commit and
    its copy of `Release`, so it cannot pick up a fix, and the assets already
    built are kept for a day only — after that, re-run all jobs.
  - A fault a fix must correct, with no published release: the tag cannot stay,
    nor a draft carrying it. Delete the tag (`git push origin :refs/tags/<tag>`)
    and, if `gh release view` showed a draft, the draft (`gh release delete
    <tag>`; `gh release list` shows drafts), then push a new tag on a commit
    carrying the fix and the tagged version.
