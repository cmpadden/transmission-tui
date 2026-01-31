use serde::Deserialize;
use serde_json::{json, Map, Value};

#[derive(Debug, Clone)]
pub struct DaemonPreferences {
    pub download_dir: String,
    pub start_when_added: bool,
    pub speed_limit_up_enabled: bool,
    pub speed_limit_up: u32,
    pub speed_limit_down_enabled: bool,
    pub speed_limit_down: u32,
    pub seed_ratio_limited: bool,
    pub seed_ratio_limit: f64,
    pub idle_seeding_limit_enabled: bool,
    pub idle_seeding_limit: u32,
    pub peer_limit_per_torrent: u32,
    pub peer_limit_global: u32,
    pub encryption_mode: EncryptionMode,
    pub pex_enabled: bool,
    pub dht_enabled: bool,
    pub lpd_enabled: bool,
    pub blocklist_enabled: bool,
    pub blocklist_url: Option<String>,
}

impl DaemonPreferences {
    pub fn to_rpc_map(&self) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "download-dir".to_string(),
            Value::String(self.download_dir.clone()),
        );
        args.insert(
            "start-added-torrents".to_string(),
            Value::Bool(self.start_when_added),
        );
        args.insert(
            "speed-limit-up-enabled".to_string(),
            Value::Bool(self.speed_limit_up_enabled),
        );
        args.insert("speed-limit-up".to_string(), json!(self.speed_limit_up));
        args.insert(
            "speed-limit-down-enabled".to_string(),
            Value::Bool(self.speed_limit_down_enabled),
        );
        args.insert("speed-limit-down".to_string(), json!(self.speed_limit_down));
        args.insert(
            "seedRatioLimited".to_string(),
            Value::Bool(self.seed_ratio_limited),
        );
        args.insert("seedRatioLimit".to_string(), json!(self.seed_ratio_limit));
        args.insert(
            "idle-seeding-limit-enabled".to_string(),
            Value::Bool(self.idle_seeding_limit_enabled),
        );
        args.insert(
            "idle-seeding-limit".to_string(),
            json!(self.idle_seeding_limit),
        );
        args.insert(
            "peer-limit-per-torrent".to_string(),
            json!(self.peer_limit_per_torrent),
        );
        args.insert(
            "peer-limit-global".to_string(),
            json!(self.peer_limit_global),
        );
        args.insert(
            "encryption".to_string(),
            Value::String(self.encryption_mode.rpc_value().to_string()),
        );
        args.insert("pex-enabled".to_string(), Value::Bool(self.pex_enabled));
        args.insert("dht-enabled".to_string(), Value::Bool(self.dht_enabled));
        args.insert("lpd-enabled".to_string(), Value::Bool(self.lpd_enabled));
        args.insert(
            "blocklist-enabled".to_string(),
            Value::Bool(self.blocklist_enabled),
        );
        args.insert(
            "blocklist-url".to_string(),
            Value::String(self.blocklist_url.clone().unwrap_or_default()),
        );
        args
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncryptionMode {
    #[default]
    Prefer,
    Allow,
    Require,
}

impl EncryptionMode {
    pub fn label(self) -> &'static str {
        match self {
            EncryptionMode::Prefer => "Prefer encryption",
            EncryptionMode::Allow => "Allow encryption",
            EncryptionMode::Require => "Require encryption",
        }
    }

    pub fn rpc_value(self) -> &'static str {
        match self {
            EncryptionMode::Prefer => "preferred",
            EncryptionMode::Allow => "tolerated",
            EncryptionMode::Require => "required",
        }
    }

    pub fn from_rpc(value: &str) -> Self {
        match value {
            "required" => EncryptionMode::Require,
            "tolerated" => EncryptionMode::Allow,
            _ => EncryptionMode::Prefer,
        }
    }

    pub fn values() -> &'static [EncryptionMode] {
        &[
            EncryptionMode::Prefer,
            EncryptionMode::Allow,
            EncryptionMode::Require,
        ]
    }
}

#[derive(Debug, Deserialize)]
pub struct PreferencesResponse {
    #[serde(rename = "download-dir")]
    download_dir: Option<String>,
    #[serde(rename = "start-added-torrents")]
    start_added: Option<bool>,
    #[serde(rename = "speed-limit-up")]
    speed_limit_up: Option<i64>,
    #[serde(rename = "speed-limit-up-enabled")]
    speed_limit_up_enabled: Option<bool>,
    #[serde(rename = "speed-limit-down")]
    speed_limit_down: Option<i64>,
    #[serde(rename = "speed-limit-down-enabled")]
    speed_limit_down_enabled: Option<bool>,
    #[serde(rename = "seedRatioLimited")]
    seed_ratio_limited: Option<bool>,
    #[serde(rename = "seedRatioLimit")]
    seed_ratio_limit: Option<f64>,
    #[serde(rename = "idle-seeding-limit-enabled")]
    idle_limit_enabled: Option<bool>,
    #[serde(rename = "idle-seeding-limit")]
    idle_limit: Option<i64>,
    #[serde(rename = "peer-limit-per-torrent")]
    peer_limit_per_torrent: Option<i64>,
    #[serde(rename = "peer-limit-global")]
    peer_limit_global: Option<i64>,
    #[serde(rename = "encryption")]
    encryption: Option<String>,
    #[serde(rename = "pex-enabled")]
    pex_enabled: Option<bool>,
    #[serde(rename = "dht-enabled")]
    dht_enabled: Option<bool>,
    #[serde(rename = "lpd-enabled")]
    lpd_enabled: Option<bool>,
    #[serde(rename = "blocklist-enabled")]
    blocklist_enabled: Option<bool>,
    #[serde(rename = "blocklist-url")]
    blocklist_url: Option<String>,
}

impl From<PreferencesResponse> for DaemonPreferences {
    fn from(value: PreferencesResponse) -> Self {
        Self {
            download_dir: value.download_dir.unwrap_or_default(),
            start_when_added: value.start_added.unwrap_or(true),
            speed_limit_up_enabled: value.speed_limit_up_enabled.unwrap_or(false),
            speed_limit_up: value.speed_limit_up.unwrap_or(0).max(0) as u32,
            speed_limit_down_enabled: value.speed_limit_down_enabled.unwrap_or(false),
            speed_limit_down: value.speed_limit_down.unwrap_or(0).max(0) as u32,
            seed_ratio_limited: value.seed_ratio_limited.unwrap_or(false),
            seed_ratio_limit: value.seed_ratio_limit.unwrap_or(2.0),
            idle_seeding_limit_enabled: value.idle_limit_enabled.unwrap_or(false),
            idle_seeding_limit: value.idle_limit.unwrap_or(30).max(0) as u32,
            peer_limit_per_torrent: value.peer_limit_per_torrent.unwrap_or(50).max(0) as u32,
            peer_limit_global: value.peer_limit_global.unwrap_or(200).max(0) as u32,
            encryption_mode: value
                .encryption
                .as_deref()
                .map(EncryptionMode::from_rpc)
                .unwrap_or_default(),
            pex_enabled: value.pex_enabled.unwrap_or(true),
            dht_enabled: value.dht_enabled.unwrap_or(true),
            lpd_enabled: value.lpd_enabled.unwrap_or(true),
            blocklist_enabled: value.blocklist_enabled.unwrap_or(false),
            blocklist_url: value.blocklist_url.filter(|s| !s.is_empty()),
        }
    }
}

pub const PREFERENCE_FIELDS: &[&str] = &[
    "download-dir",
    "start-added-torrents",
    "speed-limit-up",
    "speed-limit-up-enabled",
    "speed-limit-down",
    "speed-limit-down-enabled",
    "seedRatioLimited",
    "seedRatioLimit",
    "idle-seeding-limit-enabled",
    "idle-seeding-limit",
    "peer-limit-per-torrent",
    "peer-limit-global",
    "encryption",
    "pex-enabled",
    "dht-enabled",
    "lpd-enabled",
    "blocklist-enabled",
    "blocklist-url",
];
