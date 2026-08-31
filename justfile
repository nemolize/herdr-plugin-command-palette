# CI calls these recipes one per job, so they are the single definition of what
# each check is — there is no second copy in the workflows to drift from.

default: ci

# What CI's three jobs cover, in one command for local use
ci: lint test build-musl

fmt:
    cargo fmt --all

# CI's Lint job
lint: fmt-check clippy

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --locked --all-targets -- -D warnings

# CI's Test job
test:
    cargo test --locked

# CI's Build job — the only check that compiles the release profile
build-musl:
    rustup target add x86_64-unknown-linux-musl
    cargo build --release --locked --target x86_64-unknown-linux-musl

# Not part of `ci`: it needs the advisory database over the network, and it is what audit.yml runs on its own schedule
deny:
    cargo deny --locked check
