.PHONY: all fmt fmt-check check clippy test ci ci-local clean-clone

all: ci

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check:
	cargo check --workspace --all-targets

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

ci: fmt-check check clippy test

# Full local CI: Rust checks plus binary, artifact, eval, wasm and browser
# end-to-end steps; see scripts/ci_local.sh. ARGS=--with-smoke-training
# trains the smoke policy if artifacts/smoke_20bb.pokerbot is absent.
ci-local:
	scripts/ci_local.sh $(ARGS)

# ci-local in a fresh `git archive HEAD` export with its own target dir
# (deleted afterwards). ARGS=--include-working-tree checks uncommitted work.
clean-clone:
	scripts/clean_clone_check.sh $(ARGS)
