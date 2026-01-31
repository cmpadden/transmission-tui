# Changelog

All notable changes to this project will be documented in this file.

## [0.0.6](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.6)

- Peer pane columns expanded with an `Enc` flag and the RPC now includes peer encryption state alongside live rates.
- Added a `DD` shortcut that mirrors Transmission web's “trash data” remove action, plus a help overlay rendered as a table of key bindings.
- Torrent list now includes a Ratio column so seeding progress is visible without opening the details pane.

## [0.0.5](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.5)

- Torrent overview now displays ETA and reorders the Progress column to show Status, DL, UL, Progress, ETA for quicker status reads.
- Details pane metadata now renders in a tidy key/value table with trimmed values for easier scanning.

## [0.0.4](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.4)

- Table-based torrent list with headers (Name/Status/Progress/DL/UL) for consistent alignment and readability.
- JSON-RPC 2.0 support with automatic fallback to Transmission's legacy RPC dialect when needed.
- Peer detail table in the Details pane showing per-peer address, client, progress, and transfer rates.
- Session preference RPC keys now use snake_case and accept the new encryption strings to match Transmission 4.1+.
- Details pane now wraps text, spaces the peers section, and aligns rate/progress fields to match the torrent list formatting.

## [0.0.3](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.3)

- Preferences overlay now groups fields into titled sections (Downloading, Seeding, Speed Limits, Connections, Encryption Options, Blocklist) while keeping selection order intact for smoother navigation.

## [0.0.2](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.2)

- Add an in-app Preferences overlay (`o`) that fetches and edits Transmission's daemon settings (download directory, limits, seeding rules, discovery toggles, blocklist, etc.) directly via the RPC API.
- Drop the experimental `preferences.toml` loader so the daemon's `settings.json` remains the single source of truth.
- Document the new workflow and wire the RPC client/app state to load, cache, and save session preferences with inline validation and status feedback.

## [0.0.1](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.1)

- Initial Ratatui-powered terminal UI for monitoring and controlling `transmission-daemon` sessions.
- Async RPC worker thread keeps the interface responsive while configurable CLI/env/config settings wire up to the daemon.
- Includes session status bar, scrollable torrent list with filtering, detailed torrent pane, inline magnet prompt, and the first round of key bindings.
