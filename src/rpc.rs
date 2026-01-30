use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

use anyhow::Result;
use reqwest::{blocking::Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::{
    config::RpcConfig,
    model::{Snapshot, TorrentSummary},
};

#[derive(Debug, Error)]
pub enum TransmissionError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("authentication failed")]
    Authentication,
    #[error("session negotiation failed")]
    Session,
    #[error("unexpected http status {0}")]
    HttpStatus(StatusCode),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("response parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

pub type RpcResult<T> = std::result::Result<T, TransmissionError>;

pub struct TransmissionClient {
    http: Client,
    endpoint: String,
    auth: Option<(String, Option<String>)>,
    session_id: Mutex<Option<String>>,
    counter: AtomicU64,
}

impl TransmissionClient {
    pub fn new(config: RpcConfig) -> Result<Self> {
        let endpoint = config.endpoint();
        let RpcConfig {
            username,
            password,
            timeout,
            verify_ssl,
            user_agent,
            ..
        } = config;
        let mut builder = Client::builder().timeout(timeout).user_agent(user_agent);
        if !verify_ssl {
            builder = builder.danger_accept_invalid_certs(true);
        }
        let http = builder.build()?;
        let auth = username.map(|user| (user, password));
        Ok(Self {
            http,
            endpoint,
            auth,
            session_id: Mutex::new(None),
            counter: AtomicU64::new(1),
        })
    }

    pub fn fetch_snapshot(&self) -> RpcResult<Snapshot> {
        let fields = [
            "id",
            "name",
            "status",
            "percentDone",
            "percent_done",
            "rateDownload",
            "rate_download",
            "rateUpload",
            "rate_upload",
            "eta",
            "uploadRatio",
            "upload_ratio",
            "sizeWhenDone",
            "size_when_done",
            "leftUntilDone",
            "left_until_done",
            "downloadDir",
            "download_dir",
            "peersConnected",
            "peers_connected",
            "peersSendingToUs",
            "peers_sending_to_us",
            "peersGettingFromUs",
            "peers_getting_from_us",
            "errorString",
            "error_string",
        ];
        let torrents: TorrentGetResponse = self.torrent_get(&fields)?;
        let stats: SessionStats = self.session_stats()?;
        let session: SessionInfo = self.session_get(&["version"])?;
        Ok(Snapshot {
            version: session.version.unwrap_or_else(|| "unknown".to_string()),
            download_speed: stats.download_speed,
            upload_speed: stats.upload_speed,
            active_torrents: stats.active_torrent_count,
            paused_torrents: stats.paused_torrent_count,
            total_torrents: stats.torrent_count,
            torrents: torrents
                .torrents
                .into_iter()
                .map(TorrentSummary::from)
                .collect(),
        })
    }

    pub fn add_magnet(&self, magnet: &str) -> RpcResult<AddTorrentOutcome> {
        let args = json!({
            "filename": magnet,
        });
        let response: AddTorrentResponse = self.call("torrent-add", Some(args))?;
        Ok(AddTorrentOutcome::from(response))
    }

    pub fn remove_torrents(&self, ids: &[i64], delete_local_data: bool) -> RpcResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let args = json!({
            "ids": ids,
            "delete-local-data": delete_local_data,
        });
        self.call_raw("torrent-remove", Some(args))?;
        Ok(())
    }

    pub fn start_torrents(&self, ids: &[i64]) -> RpcResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let args = json!({ "ids": ids });
        self.call_raw("torrent-start", Some(args))?;
        Ok(())
    }

    pub fn stop_torrents(&self, ids: &[i64]) -> RpcResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let args = json!({ "ids": ids });
        self.call_raw("torrent-stop", Some(args))?;
        Ok(())
    }

    fn session_get<T>(&self, fields: &[&str]) -> RpcResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let args = if fields.is_empty() {
            None
        } else {
            Some(json!({"fields": fields}))
        };
        let value = self.call_raw("session-get", args)?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn session_stats(&self) -> RpcResult<SessionStats> {
        let value = self.call_raw("session-stats", None)?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn torrent_get(&self, fields: &[&str]) -> RpcResult<TorrentGetResponse> {
        let args = json!({"fields": fields});
        let value = self.call_raw("torrent-get", Some(args))?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn call<T>(&self, method: &str, arguments: Option<Value>) -> RpcResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let value = self.call_raw(method, arguments)?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn call_raw(&self, method: &str, arguments: Option<Value>) -> RpcResult<Value> {
        let payload = RpcRequest {
            method,
            arguments,
            tag: self.counter.fetch_add(1, Ordering::Relaxed),
        };
        loop {
            let mut request = self
                .http
                .post(&self.endpoint)
                .header("Content-Type", "application/json")
                .json(&payload);
            if let Some((user, pass)) = &self.auth {
                request = request.basic_auth(user, pass.as_ref());
            }
            let session_header = match self.session_id.lock() {
                Ok(guard) => (*guard).clone(),
                Err(_) => None,
            };
            if let Some(session) = session_header {
                request = request.header("X-Transmission-Session-Id", session);
            }
            let response = request.send()?;
            match response.status() {
                StatusCode::CONFLICT => {
                    if let Some(id) = response.headers().get("X-Transmission-Session-Id") {
                        let value = id
                            .to_str()
                            .map_err(|_| TransmissionError::Session)?
                            .to_string();
                        if let Ok(mut guard) = self.session_id.lock() {
                            *guard = Some(value);
                        }
                        continue;
                    }
                    return Err(TransmissionError::Session);
                }
                StatusCode::UNAUTHORIZED => return Err(TransmissionError::Authentication),
                status if !status.is_success() => {
                    return Err(TransmissionError::HttpStatus(status));
                }
                _ => {
                    let body: RpcResponse = response.json()?;
                    if body.result != "success" {
                        return Err(TransmissionError::Rpc(body.result));
                    }
                    return Ok(body.arguments.unwrap_or(Value::Null));
                }
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<Value>,
    tag: u64,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    #[serde(default)]
    arguments: Option<Value>,
    result: String,
}

#[derive(Debug, Deserialize)]
struct SessionStats {
    #[serde(default, alias = "activeTorrentCount")]
    active_torrent_count: i64,
    #[serde(default, alias = "pausedTorrentCount")]
    paused_torrent_count: i64,
    #[serde(default, alias = "torrentCount")]
    torrent_count: i64,
    #[serde(default, alias = "downloadSpeed")]
    download_speed: i64,
    #[serde(default, alias = "uploadSpeed")]
    upload_speed: i64,
}

#[derive(Debug, Deserialize)]
struct SessionInfo {
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TorrentGetResponse {
    #[serde(default)]
    torrents: Vec<TorrentWire>,
}

#[derive(Debug, Deserialize)]
struct TorrentWire {
    #[serde(alias = "id")]
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: i64,
    #[serde(default, alias = "percentDone")]
    percent_done: f64,
    #[serde(default, alias = "rateDownload")]
    rate_download: i64,
    #[serde(default, alias = "rateUpload")]
    rate_upload: i64,
    #[serde(default)]
    eta: i64,
    #[serde(default, alias = "uploadRatio")]
    upload_ratio: f64,
    #[serde(default, alias = "sizeWhenDone")]
    size_when_done: i64,
    #[serde(default, alias = "leftUntilDone")]
    left_until_done: i64,
    #[serde(default, alias = "downloadDir")]
    download_dir: String,
    #[serde(default, alias = "peersConnected")]
    peers_connected: i64,
    #[serde(default, alias = "peersSendingToUs")]
    peers_sending_to_us: i64,
    #[serde(default, alias = "peersGettingFromUs")]
    peers_getting_from_us: i64,
    #[serde(default, alias = "errorString")]
    error_string: String,
}

impl From<TorrentWire> for TorrentSummary {
    fn from(wire: TorrentWire) -> Self {
        let eta = if wire.eta >= 0 { Some(wire.eta) } else { None };
        let status = match wire.status {
            0 => "stopped",
            1 => "check-wait",
            2 => "checking",
            3 => "download-wait",
            4 => "downloading",
            5 => "seed-wait",
            6 => "seeding",
            other => {
                return TorrentSummary {
                    torrent_id: wire.id,
                    name: wire.name,
                    status: format!("status-{}", other),
                    percent_done: wire.percent_done,
                    rate_download: wire.rate_download,
                    rate_upload: wire.rate_upload,
                    eta,
                    upload_ratio: wire.upload_ratio,
                    size_when_done: wire.size_when_done,
                    left_until_done: wire.left_until_done,
                    download_dir: wire.download_dir,
                    peers_connected: wire.peers_connected,
                    peers_sending: wire.peers_sending_to_us,
                    peers_receiving: wire.peers_getting_from_us,
                    error: if wire.error_string.is_empty() {
                        None
                    } else {
                        Some(wire.error_string)
                    },
                }
            }
        };
        TorrentSummary {
            torrent_id: wire.id,
            name: wire.name,
            status: status.to_string(),
            percent_done: wire.percent_done,
            rate_download: wire.rate_download,
            rate_upload: wire.rate_upload,
            eta,
            upload_ratio: wire.upload_ratio,
            size_when_done: wire.size_when_done,
            left_until_done: wire.left_until_done,
            download_dir: wire.download_dir,
            peers_connected: wire.peers_connected,
            peers_sending: wire.peers_sending_to_us,
            peers_receiving: wire.peers_getting_from_us,
            error: if wire.error_string.is_empty() {
                None
            } else {
                Some(wire.error_string)
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct AddTorrentResponse {
    #[serde(rename = "torrent-added", alias = "torrentAdded")]
    torrent_added: Option<TorrentRef>,
    #[serde(rename = "torrent-duplicate", alias = "torrentDuplicate")]
    torrent_duplicate: Option<TorrentRef>,
}

#[derive(Debug, Deserialize)]
struct TorrentRef {
    id: Option<i64>,
    name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddTorrentOutcome {
    pub torrent_id: Option<i64>,
    pub name: Option<String>,
    pub added: bool,
    pub duplicate: bool,
}

impl From<AddTorrentResponse> for AddTorrentOutcome {
    fn from(resp: AddTorrentResponse) -> Self {
        if let Some(added) = resp.torrent_added {
            AddTorrentOutcome {
                torrent_id: added.id,
                name: added.name,
                added: true,
                duplicate: false,
            }
        } else if let Some(dup) = resp.torrent_duplicate {
            AddTorrentOutcome {
                torrent_id: dup.id,
                name: dup.name,
                added: false,
                duplicate: true,
            }
        } else {
            AddTorrentOutcome {
                torrent_id: None,
                name: None,
                added: false,
                duplicate: false,
            }
        }
    }
}
