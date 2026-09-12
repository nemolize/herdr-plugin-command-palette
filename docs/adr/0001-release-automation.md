# ADR 0001 — Cutting releases with release-please, without a PAT or a GitHub App

Status: accepted (2026-09-05)

`docs/ci.md` describes how releases are cut today. This records what was
rejected on the way there, which that document does not carry and a reader
cannot recover from the workflows: every alternative below leaves no trace in
the files that were kept.

## Context

Releases were hand-tagged. The maintainer wanted the tag chosen and pushed
automatically, and a changelog written without hand-editing one.

Two GitHub behaviours constrain every option:

- A tag or pull request the default `GITHUB_TOKEN` writes **starts no workflow
  run**. Whatever cuts the tag cannot, by that route alone, make the five
  release assets build.
- Workflow runs on a pull request that token opens are created in an
  approval-required state. They exist, but sit behind a click.

The first of these is what most of the decision turns on.

## Decision 1 — release-please over changesets

Three candidates were compared.

| Option | Rejected because |
|---|---|
| Date-stamped tag from a `workflow_dispatch` button | Cuts a tag but writes no changelog, which was half the goal |
| **release-please** | **Chosen** |
| changesets | Adds a `.changeset/*.md` per pull request — new manual work, to remove manual work |

The deciding argument against changesets is that it is a tool for deciding a
**package's** version under SemVer, and nothing here consumes the version that
way. `herdr/install.sh` reads it out of the manifest and interpolates it into a
download URL — no parse, no range check — and no package manager resolves this
plugin against a version range. Where the major/minor/patch judgement reaches
nothing that acts on it, paying a per-PR file to make it deliberately is cost
without return.

release-please derives the same judgement from commit messages the repository
already writes (`feat:`, `fix:`), so it costs nothing per pull request.

## Decision 2 — `workflow_call`, not a PAT and not a GitHub App

`Release-Please` invokes `Release` through `workflow_call` rather than pushing a
tag and leaving the tag's own `push` trigger to start the build. A
`workflow_call` is an invocation rather than an event, so the `GITHUB_TOKEN`
restriction above does not apply to it, and no second credential is needed.

`banaris/website` faced the same restriction and resolved it with a GitHub App,
after weighing a PAT (ties the release to one person's account and expires) and
relaxing the branch ruleset (lowers what protects `main`). What made the
credential hard to avoid there is that its tag has to start a deployment, and a
deployment triggered by an event cannot be reached by an invocation.

`Release` is a separate workflow file here too, but it was written to accept
`workflow_call`, so `Release-Please` invokes it rather than waiting for an event
that will not arrive. That is the axis the comparison turns on — not how many
files there are, but whether the thing to be started is callable at all. Which
is why this repository holds no App registration, no secret, and no expiry to
renew.

Rejected alongside: teaching `Release` to accept `repository_dispatch` (the same
indirection, with the tag-manifest agreement harder to assert), and having
release-please write a draft release for a human to publish (a second manual
step, when removing one was the point).

## Decision 3 — the release PR's approval click is kept

`Lint`, `Test` and `Build` are required by a ruleset, and on the release PR they
sit unapproved until someone clicks *Approve workflows to run* — the
approval-required state above. Two ways out were available and both were
rejected:

- **Exclude the release PR from the required checks**, or drop the requirement.
  Rejected: it lowers what protects `main` to save one click, and the checks are
  required precisely so a release cannot go out on an untested tree.
- **Author the PR with a PAT or an App token**, so its runs start normally.
  Rejected: it reintroduces the credential decision 2 avoids, for a click.

The click is treated as the release's own review step. A release PR is the one
pull request nobody else reviews, so a deliberate human action before the assets
build is not an imposition on the flow — it is the only one the flow has.

The admin role can bypass the ruleset via pull request, which would let a
release PR merge with its runs never approved. Decision 3 is the choice not to
take that route routinely: approving the runs and merging on green is the flow,
and the bypass is what remains available when it cannot be.

## Consequences

- Every release costs one approval click, on the release PR and on each re-sync
  as `main` moves. This is accepted, not a defect to route around — an issue
  proposing to remove it should be read against decision 3 first.
- No credential exists to rotate, expire, or leak, and no organisation-level App
  registration is required to fork this configuration into another repository.
- The `workflow_call` route holds only while the build is reachable by
  invocation. Moving asset builds behind something that must be triggered as an
  event would bring back the constraint, and with it the credential decision 2
  avoided.
- Commit messages are load-bearing, and the version and the changelog read them
  by different rules. Anything neither breaking nor `feat:` bumps the patch, so
  a `feat:` written as `chore:` costs the minor bump; but `chore:` writes no
  changelog entry, and a release whose notes come out empty is skipped
  altogether. Mislabel the only feature in a cycle and it ships as a patch
  nobody can see, or does not ship at all.

## Provenance

Decision 1's option comparison and decision 2's credential weighing were worked
through for `banaris/website` on 2026-08-26 and 2026-08-28, and are reused here
rather than re-derived; the `workflow_call` route is this repository's own and
its reasoning is recorded in `docs/ci.md` under "Cutting a release".
