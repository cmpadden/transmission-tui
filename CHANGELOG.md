# Changelog

All notable changes to this project will be documented in this file.

## [0.0.2](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.2)

- Add an in-app Preferences overlay (`o`) that fetches and edits Transmission's daemon settings (download directory, limits, seeding rules, discovery toggles, blocklist, etc.) directly via the RPC API.
- Drop the experimental `preferences.toml` loader so the daemon's `settings.json` remains the single source of truth.
- Document the new workflow and wire the RPC client/app state to load, cache, and save session preferences with inline validation and status feedback.

## [0.0.1](https://github.com/cmpadden/transmission-tui/releases/tag/v0.0.1)

- Initial Ratatui-powered terminal UI for monitoring and controlling `transmission-daemon` sessions.
- Async RPC worker thread keeps the interface responsive while configurable CLI/env/config settings wire up to the daemon.
- Includes session status bar, scrollable torrent list with filtering, detailed torrent pane, inline magnet prompt, and the first round of key bindings.
