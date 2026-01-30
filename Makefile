.PHONY: build run fmt lint check clean

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
