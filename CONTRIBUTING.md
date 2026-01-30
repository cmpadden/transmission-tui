# Contributing

## Building & Running

Rust 1.83+ with Cargo is required. Typical flows:

```bash
# Build locally
cargo build

# Run with CLI flags
cargo run -- --host localhost --port 9091

# Release binary
cargo build --release
./target/release/transmission-tui --url http://localhost:9091/transmission/rpc
```

Convenience targets are available via `make build`, `make run ARGS="--host ..."`, `make fmt`, `make lint`, and `make check`.

## Architecture

- `config.rs`: CLI & env parsing plus TOML config ingestion.
- `rpc.rs`: JSON-RPC client with session-ID negotiation, request helpers, and Transmission-specific data models.
- `tui.rs`: Ratatui widgets, keyboard handling, event loop, and worker threads for RPC + input.
- `model.rs`: Shared snapshot/torrent summary types and display helpers.

Background RPC work is offloaded to a channel-driven thread. It polls at the configured interval, handles command requests (refresh, add magnet), and streams results back to the UI via lightweight events. Input events are read on a separate thread so the Ratatui render loop never blocks on network or keyboard I/O.

The Transmission RPC reference used by the client lives in `docs/RPC_REFERENCE.md`.

## Development Tips

- `make fmt` (rustfmt) and `make lint` (clippy `-D warnings`) keep the code tidy.
- `make check` runs the test suite (currently placeholder until RPC mocks are added).
- The app logs through `env_logger`. Set `RUST_LOG=transmission_tui=debug` for verbose RPC traces.
- When hacking on the RPC layer, use `docs/RPC_REFERENCE.md` as the single source of truth for payloads and expected responses.
