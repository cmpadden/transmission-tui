use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub version: String,
    pub download_speed: i64,
    pub upload_speed: i64,
    pub active_torrents: i64,
    pub paused_torrents: i64,
    pub total_torrents: i64,
    pub torrents: Vec<TorrentSummary>,
}

#[derive(Debug, Clone)]
pub struct TorrentSummary {
    pub torrent_id: i64,
    pub name: String,
    pub status: String,
    pub percent_done: f64,
    pub rate_download: i64,
    pub rate_upload: i64,
    pub eta: Option<i64>,
    pub upload_ratio: f64,
    pub size_when_done: i64,
    pub left_until_done: i64,
    pub download_dir: String,
    pub peers_connected: i64,
    pub peers_sending: i64,
    pub peers_receiving: i64,
    pub error: Option<String>,
    pub peers: Vec<PeerSummary>,
}

#[derive(Debug, Clone)]
pub struct PeerSummary {
    pub address: String,
    pub client: String,
    pub progress: f64,
    pub rate_down: i64,
    pub rate_up: i64,
}

pub fn format_speed(value: i64) -> String {
    const UNITS: [&str; 5] = ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s"];
    let mut magnitude = value.max(0) as f64;
    let mut unit = 0;
    while magnitude >= 1024.0 && unit < UNITS.len() - 1 {
        magnitude /= 1024.0;
        unit += 1;
    }
    format!("{:>4.1}{}", magnitude, UNITS[unit])
}

pub fn format_progress(value: f64) -> String {
    format!("{:5.1}%", value * 100.0)
}

pub fn format_eta(seconds: Option<i64>) -> String {
    match seconds {
        None => "∞".to_string(),
        Some(raw) if raw < 0 => "∞".to_string(),
        Some(raw) => {
            let duration = Duration::from_secs(raw as u64);
            let days = duration.as_secs() / 86_400;
            let hours = (duration.as_secs() % 86_400) / 3_600;
            let minutes = (duration.as_secs() % 3_600) / 60;
            let seconds = duration.as_secs() % 60;
            if days > 0 {
                format!("{}d{}h", days, hours)
            } else if hours > 0 {
                format!("{}h{}m", hours, minutes)
            } else if minutes > 0 {
                format!("{}m", minutes)
            } else {
                format!("{}s", seconds)
            }
        }
    }
}

pub fn format_bytes(value: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut magnitude = value.max(0) as f64;
    let mut unit = 0;
    while magnitude >= 1024.0 && unit < UNITS.len() - 1 {
        magnitude /= 1024.0;
        unit += 1;
    }
    format!("{:>4.1} {}", magnitude, UNITS[unit])
}
