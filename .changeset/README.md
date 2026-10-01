# Changesets

Each file here is one pending release note. A pull request that changes what a
user of the plugin sees — a new command, a fixed behaviour, a changed default —
adds one; a change nobody installing the plugin can observe (CI, docs, tests,
refactors, dependency bumps with no behaviour change) adds none.

```sh
pnpm install --ignore-scripts
pnpm exec changeset add --minor herdr-command-palette -m "Add vertical pane swaps to the command palette"
```

Or write the file by hand, any name ending in `.md`:

```md
---
"herdr-command-palette": minor
---

Add vertical pane swaps to the command palette
```

- `minor` for a new capability, `patch` for a fix. Pre-1.0, a breaking change
  is also `minor`.
- The note is the changelog line, verbatim — write it for the person reading the
  release, not the reviewer.

Merging it to `main` opens or updates the `Version Packages` pull request.
Merging that pull request consumes every pending file, bumps the version and
drafts the release. `docs/ci.md`, "Cutting a release", has the rest.
