.PHONY: build run fmt lint check clean release

build:
	cargo build

run:
	cargo run -- $(ARGS)

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

check:
	cargo test

clean:
	cargo clean

release:
	@if [ -z "$(VERSION)" ]; then \
		echo "VERSION is required, e.g. make release VERSION=0.0.4"; \
		exit 1; \
	fi
	gh workflow run release.yml -f version=$(VERSION)
