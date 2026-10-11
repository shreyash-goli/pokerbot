.PHONY: all fmt fmt-check check clippy test validate ci

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

validate:
	python3 tools/validate_tasks.py
	python3 -m unittest discover -s tests -v

ci: fmt-check check clippy test validate
