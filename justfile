# CI runs `just ci` and `just deny`, so these recipes are the single definition
# of what the checks are — there is no second copy in the workflows to drift from.

default: ci

ci: fmt-check clippy test build-musl

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --locked --all-targets -- -D warnings

test:
    cargo test --locked

# Exercises [profile.release] and .cargo/config.toml's rust-lld pin, which the test profile never compiles
build-musl:
    rustup target add x86_64-unknown-linux-musl
    cargo build --release --locked --target x86_64-unknown-linux-musl

# Not part of `ci`: it needs the advisory database over the network, and it is what audit.yml runs on its own schedule
deny:
    cargo deny --locked check
