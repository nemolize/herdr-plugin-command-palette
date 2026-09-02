# The workflows call these recipes rather than spelling out the commands, so
# each check and build has one definition with no second copy to drift from.

default: ci

ci: lint test build-musl

fmt:
    cargo fmt --all

lint: fmt-check clippy

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --locked --all-targets -- -D warnings

test:
    cargo test --locked

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

# Not part of `ci`: it needs a herdr binary, which the Catalog job fetches
# rather than every other job carrying that cost.
catalog-e2e:
    python3 herdr/catalog-e2e.py
