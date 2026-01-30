# transmission-tui

A [Ratatui](https://github.com/ratatui/ratatui)-based terminal UI for managing `transmission-daemon` instances.

```
┌ Session ─────────────────────────────────────────────────────────────────────────────────┐
│Transmission  |  http://localhost:9091/transmission/rpc                                   │
│DL  0.0B/s  UL  1.4KiB/s  | Active 1  Paused 1  Total 2  | Version 4.0.6 (38c164933e)     │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌ Torrents ────────────────────────────────────────────────────────────────────────────────┐
│> archlinux-2026.01.01-x86_64.iso           seeding      100.0%  DL  0.0B/s  UL 1000.0B/s │
│  archlinux-2025.12.01-x86_64.iso           stopped        0.0%  DL  0.0B/s  UL  0.0B/s   │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
┌ Details ─────────────────────────────────────────────────────────────────────────────────┐
│archlinux-2026.01.01-x86_64.iso                                                           │
│Status: seeding                                                                           │
│Progress: 100.0%  ETA ∞                                                                   │
│Size:  1.4 GiB (remaining  0.0 B)                                                         │
│Rates: DL  0.0B/s  UL 1000.0B/s                                                           │
│Ratio: 0.01                                                                               │
│Peers: sending 0 | receiving 1 | connected 50                                             │
└──────────────────────────────────────────────────────────────────────────────────────────┘
Mode NORMAL | Filter (no filter)                                                    Help [?]
```

## Features

- Async-friendly RPC worker thread that keeps the UI responsive while polling the daemon.
- Configurable connection settings via CLI flags, environment variables, or a `$XDG_CONFIG_HOME/transmission-tui/config.toml` file.
- Session status bar showing live download/upload speeds, torrent counts, and alert messages.
- Scrollable torrent list with filtering, sorting preservation, and focus retention when new torrents arrive.
- Detail pane with progress, ETA, transfer rates, ratios, peer counts, download path, and error text.
- Inline magnet prompt with automatic focus on the added/duplicate torrent once the daemon responds.

## Configuration

Settings are resolved in this order: CLI flag → environment variable → config file → default.

| CLI flag | Environment variable | Description |
| --- | --- | --- |
| `--url` | `TRANSMISSION_URL` | Full RPC URL; overrides host/port/path |
| `--host` | `TRANSMISSION_HOST` | Daemon host (default `localhost`) |
| `--port` | `TRANSMISSION_PORT` | Daemon port (default `9091`) |
| `--path` | `TRANSMISSION_RPC_PATH` | RPC path (default `/transmission/rpc`) |
| `--username` | `TRANSMISSION_USERNAME` | Basic auth username |
| `--password` | `TRANSMISSION_PASSWORD` | Basic auth password |
| `--timeout` | `TRANSMISSION_TIMEOUT` | HTTP timeout in seconds (default `10`) |
| `--poll-interval` | `TRANSMISSION_POLL_INTERVAL` | Background refresh cadence in seconds (default `3`) |
| `--tls/--no-tls` | `TRANSMISSION_TLS` | Force HTTPS on/off (default HTTP) |
| `--insecure` | `TRANSMISSION_VERIFY_SSL=0` | Disable TLS verification |
| `--log-level` | `TRANSMISSION_LOG_LEVEL` | `trace`, `debug`, `info`, etc. |

Config file example (`$XDG_CONFIG_HOME/transmission-tui/config.toml`):

```toml
[rpc]
host = "nas.lan"
port = 9091
username = "media"
password = "secret"
tls = true
verify_ssl = false
poll_interval = 2.5
```

## Key Bindings

- `j` / `k`: Move selection down/up
- `g` / `G`: Jump to top/bottom
- `Ctrl+d` / `Ctrl+u`: Half-page scroll
- `/`: Inline name filter (type + Enter, Esc to cancel)
- `r`: Resume/start the selected torrent
- `p`: Pause the selected torrent
- `R`: Manual refresh (in addition to the background poller)
- `a`: Add magnet link (paste + Enter, Esc to cancel)
- `dd`: Remove the selected torrent (confirmation prompt)
- `?`: Toggle the in-app help overlay with the full binding list
- `q` or `Ctrl+c`: Quit the UI

The footer shows the current mode (NORMAL / FILTER / PROMPT / CONFIRM / HELP), the active filter string, and a `Help (?)` hint you can press anytime in normal mode.

## Contributing

See `CONTRIBUTING.md` for build instructions, architecture notes, and development tips.
