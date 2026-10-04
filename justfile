# The workflows call these recipes rather than spelling out the commands, so
# each check and build has one definition with no second copy to drift from.

default: ci

ci: lint test palette-e2e open-test catalog-e2e-test popup-collision-e2e-test release-test build-musl zizmor zizmor-test

fmt:
    cargo fmt --all

lint: fmt-check clippy

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --locked --all-targets -- -D warnings

test:
    cargo test --locked

# Without a token zizmor skips its online audits. Its default persona passes a pin
# whose comment is not a single tag, missing included; only pedantic reports that.
[positional-arguments]
zizmor path='.':
    #!/usr/bin/env bash
    set -euo pipefail
    path=$1
    if [ -z "${GH_TOKEN:-}" ]; then GH_TOKEN=$(gh auth token); export GH_TOKEN; fi
    zizmor "$path"
    filter='.[]
        | select(.ident == "ref-version-mismatch")
        | .locations[0]
        | "\(.symbolic.key.Local.verbatim_path):\(.concrete.location.start_point.row + 1): the version comment must be exactly the pinned tag"'
    unverified=$(zizmor -q --persona pedantic --format json-v1 --no-exit-codes "$path" | jq -r "$filter")
    if [ -n "$unverified" ]; then
        printf '%s\n' "$unverified" >&2
        exit 1
    fi

# The repository's own pins never trip the version-comment check, so a broken
# check would pass there unnoticed; this runs it on fixtures that must trip it.
zizmor-test:
    node --test scripts/zizmor.test.mjs

# Needs `pnpm install --ignore-scripts` first, for the Changesets CLI the lockfile pins.
release-test:
    node scripts/sync-version.mjs --check
    node --test scripts/release.test.mjs

# Part of `ci`, unlike catalog-e2e below: it stubs herdr rather than fetching
# one, so it costs a debug build and a few seconds.
palette-e2e:
    cargo build --locked
    python3 herdr/palette-e2e.py

# Part of `ci` for the same reason: the action hop runs against a stubbed herdr.
open-test:
    python3 herdr/open-test.py

# Part of `ci` too: it tests catalog-e2e's response check and its verdict,
# running the harness against a stubbed herdr rather than a fetched one.
catalog-e2e-test:
    python3 herdr/catalog-e2e-test.py

# Part of `ci` too: it tests popup-collision-e2e's verdict against stubbed runs.
popup-collision-e2e-test:
    python3 herdr/popup-collision-e2e-test.py

# One release asset. release.yml calls this per matrix target, so the build
# invocation has one definition rather than a copy per consumer.
build-release target:
    cargo build --release --locked --target {{ target }}

# Both musl release assets — the only CI check compiling the release profile.
# The release workflow installs targets via its setup action instead.
build-musl:
    rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
    just build-release x86_64-unknown-linux-musl
    just build-release aarch64-unknown-linux-musl

# Not part of `ci`: it needs the advisory database over the network, and it is what audit.yml runs on its own schedule
deny:
    cargo deny --locked check

# Fetch a herdr binary into ./bin, for the catalog E2E below.
fetch-herdr:
    sh herdr/fetch-herdr.sh ./bin

# Not part of `ci`: it needs a herdr binary, which the E2ETests job fetches
# rather than every other job carrying that cost.
catalog-e2e:
    python3 herdr/catalog-e2e.py

# Not part of `ci` for the same reason: it needs a fetched herdr too.
popup-collision-e2e:
    python3 herdr/popup-collision-e2e.py
