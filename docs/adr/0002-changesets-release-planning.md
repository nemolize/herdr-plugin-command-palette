# ADR 0002 — Planning releases with Changesets instead of release-please

Status: accepted (2026-09-29). Supersedes ADR 0001's decision 1 and its
commit-message consequence; ADR 0001's decisions 2 (`workflow_call`, no PAT or
App) and 3 (the version PR's approval click) stand.

`docs/ci.md`, "Cutting a release", describes the flow as built. This records
why release-please was replaced and what the replacement costs.

## Context

release-please failed the same way on three consecutive releases. Once a
release PR merged, it created the draft release and then — in the same run,
before the assets built and the draft was published — planned the next release.
The draft has no tag until it is published, so release-please could not find
the release it had just cut, walked back through history, and opened a
redundant release PR for commits that had already shipped (#43; v0.2.0, v0.3.0
in #57, v0.4.0 in #91).

Separately, it collected each change twice whenever a merge commit's body
repeated its PR's conventional-commit subject — the default for a merge commit
here (#41). release-please's documented remedy is squash merging.

Two repair routes were weighed against migrating:

| Route | Outcome |
|---|---|
| release-please's `force-tag-creation: true` | Rejected. Addresses #43 only, and makes the tag visible before the binaries are — the window ADR 0001's draft exists to close |
| Switch to squash merging | Rejected. Addresses #41 only, and the repository keeps ordinary merge commits, including for stacked PRs |
| **Changesets** | **Chosen** |

## Decision — explicit changeset files decide the version and the notes

A pull request that changes what a plugin user sees adds a `.changeset/*.md`
naming the bump and the changelog line. `changesets/action` keeps one version
PR open while any are pending; merging it consumes them. A separate step then
drafts the release for the version in `package.json` unless a release — draft or
published — already carries it.

Neither problem above can occur by construction: the next version comes from
unconsumed files, which the version PR deletes, not from tags or commit history,
so a draft still being built cannot make its changes look unreleased; and a
changelog entry comes from a file, so a merge commit adds none. Two files with
the same text still produce two entries — this is not general deduplication.

ADR 0001 rejected Changesets for its per-PR file: new manual work, and a
SemVer judgement nothing downstream consumes. That cost is now accepted. The
file buys what inference could not deliver — a release that contains exactly
what was declared, with notes written for the reader rather than recovered
from commit subjects — without changing the merge policy.

## Consequences

- **A `package.json` exists that publishes nothing.** Changesets versions
  packages it finds through `package.json`; its documented route for non-npm
  projects is a `private` package with `privatePackages.version` enabled. The
  file carries the version, the pinned CLI and one script. npm publication is
  not configured and must not be.
- **The version lives in four places**: `package.json`, `Cargo.toml`, the root
  package in `Cargo.lock`, and `herdr-plugin.toml`. `scripts/sync-version.mjs`
  copies the first into the rest during the version step, and
  `just release-test` fails the required `Test` job when they disagree.
- **Node joins the toolchain** for release work and for `just release-test`.
- **Rust-native versioning is given up.** release-please edited `Cargo.toml`
  and `Cargo.lock` itself; here a small script does, and it knows only the
  root crate.
- **A forgotten changeset ships nothing.** A user-visible change merged without
  one reaches no release until a later changeset does. This is the price of
  declared intent; review is what catches it.
- **Retry needs no new version.** A failed build leaves an untagged draft; the
  next `Release Plan` run — a re-run, a dispatch, or any push to `main` —
  rebuilds the commit that draft targets.
- **v0.4.0 is the baseline.** It was cut by release-please and is already
  published; the migration adds no changeset for it and cuts no release of its
  own.
