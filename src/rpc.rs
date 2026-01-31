use std::{
    borrow::Cow,
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};

use anyhow::Result;
use reqwest::{blocking::Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

use crate::{
    config::RpcConfig,
    model::{PeerSummary, Snapshot, TorrentSummary},
    preferences::{DaemonPreferences, PreferencesResponse, PREFERENCE_FIELDS},
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
    #[error("rpc error {code}: {message}{context}")]
    Rpc {
        code: i64,
        message: String,
        context: String,
    },
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
    use_json_rpc: AtomicBool,
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
            use_json_rpc: AtomicBool::new(true),
        })
    }

    pub fn fetch_preferences(&self) -> RpcResult<DaemonPreferences> {
        let prefs: PreferencesResponse = self.session_get(PREFERENCE_FIELDS)?;
        Ok(DaemonPreferences::from(prefs))
    }

    pub fn update_preferences(&self, prefs: &DaemonPreferences) -> RpcResult<()> {
        let args = Value::Object(prefs.to_rpc_map());
        self.call_raw("session_set", Some(args))?;
        Ok(())
    }

    pub fn fetch_snapshot(&self) -> RpcResult<Snapshot> {
        let fields = [
            "id",
            "name",
            "status",
            "percent_done",
            "rate_download",
            "rate_upload",
            "eta",
            "upload_ratio",
            "size_when_done",
            "left_until_done",
            "download_dir",
            "peers_connected",
            "peers_sending_to_us",
            "peers_getting_from_us",
            "error_string",
            "peers",
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
        let response: AddTorrentResponse = self.call("torrent_add", Some(args))?;
        Ok(AddTorrentOutcome::from(response))
    }

    pub fn remove_torrents(&self, ids: &[i64], delete_local_data: bool) -> RpcResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let args = json!({
            "ids": ids,
            "delete_local_data": delete_local_data,
        });
        self.call_raw("torrent_remove", Some(args))?;
        Ok(())
    }

    pub fn start_torrents(&self, ids: &[i64]) -> RpcResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let args = json!({ "ids": ids });
        self.call_raw("torrent_start", Some(args))?;
        Ok(())
    }

    pub fn stop_torrents(&self, ids: &[i64]) -> RpcResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let args = json!({ "ids": ids });
        self.call_raw("torrent_stop", Some(args))?;
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
        let value = self.call_raw("session_get", args)?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn session_stats(&self) -> RpcResult<SessionStats> {
        let value = self.call_raw("session_stats", None)?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn torrent_get(&self, fields: &[&str]) -> RpcResult<TorrentGetResponse> {
        let args = json!({"fields": fields});
        let value = self.call_raw("torrent_get", Some(args))?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn call<T>(&self, method: &'static str, arguments: Option<Value>) -> RpcResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let value = self.call_raw(method, arguments)?;
        serde_json::from_value(value).map_err(TransmissionError::from)
    }

    fn call_raw(&self, method: &'static str, arguments: Option<Value>) -> RpcResult<Value> {
        if self.use_json_rpc.load(Ordering::Relaxed) {
            match self.call_raw_inner(RpcProtocol::Json, method, arguments.clone()) {
                Ok(value) => return Ok(value),
                Err(err) if self.should_retry_in_legacy(&err) => {
                    self.use_json_rpc.store(false, Ordering::Relaxed);
                    return self.call_raw_inner(RpcProtocol::Legacy, method, arguments);
                }
                Err(err) => return Err(err),
            }
        }
        self.call_raw_inner(RpcProtocol::Legacy, method, arguments)
    }

    fn call_raw_inner(
        &self,
        protocol: RpcProtocol,
        method: &'static str,
        arguments: Option<Value>,
    ) -> RpcResult<Value> {
        let rpc_method = method_for_protocol(method, protocol);
        let params = translate_arguments_for_protocol(protocol, method, arguments);
        match protocol {
            RpcProtocol::Json => {
                let payload = JsonRpcRequest {
                    jsonrpc: "2.0",
                    method: rpc_method,
                    params,
                    id: self.counter.fetch_add(1, Ordering::Relaxed),
                };
                self.perform_request(&payload)
            }
            RpcProtocol::Legacy => {
                let payload = LegacyRpcRequest {
                    method: rpc_method,
                    arguments: params,
                    tag: self.counter.fetch_add(1, Ordering::Relaxed),
                };
                self.perform_request(&payload)
            }
        }
    }

    fn should_retry_in_legacy(&self, err: &TransmissionError) -> bool {
        match err {
            TransmissionError::Rpc { code, message, .. } => {
                let normalized = message.to_ascii_lowercase();
                *code == -32601
                    || normalized.contains("method not found")
                    || normalized.contains("method name not recognized")
            }
            _ => false,
        }
    }

    fn perform_request<T>(&self, payload: &T) -> RpcResult<Value>
    where
        T: Serialize,
    {
        loop {
            let mut request = self
                .http
                .post(&self.endpoint)
                .header("Content-Type", "application/json");
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
            let response = request.json(payload).send()?;
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
                    let body: Value = response.json()?;
                    return handle_response_body(body);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RpcProtocol {
    Json,
    Legacy,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    id: u64,
}

#[derive(Debug, Serialize)]
struct LegacyRpcRequest<'a> {
    method: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<Value>,
    tag: u64,
}

fn handle_response_body(body: Value) -> RpcResult<Value> {
    if body.get("jsonrpc").is_some() {
        handle_json_rpc_body(body)
    } else {
        handle_legacy_body(body)
    }
}

fn handle_json_rpc_body(body: Value) -> RpcResult<Value> {
    if let Some(error) = body.get("error") {
        return Err(parse_json_rpc_error(error));
    }
    Ok(body.get("result").cloned().unwrap_or(Value::Null))
}

fn handle_legacy_body(body: Value) -> RpcResult<Value> {
    let result = body
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| response_parse_error("missing legacy result"))?;
    if result != "success" {
        let context = body
            .get("arguments")
            .and_then(extract_context_from_value)
            .map(|ctx| format!(" ({ctx})"))
            .unwrap_or_default();
        return Err(TransmissionError::Rpc {
            code: -1,
            message: result.to_string(),
            context,
        });
    }
    Ok(body.get("arguments").cloned().unwrap_or(Value::Null))
}

fn parse_json_rpc_error(error: &Value) -> TransmissionError {
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("rpc error")
        .to_string();
    let context = error
        .get("data")
        .and_then(extract_context_from_value)
        .map(|ctx| format!(" ({ctx})"))
        .unwrap_or_default();
    TransmissionError::Rpc {
        code,
        message,
        context,
    }
}

fn extract_context_from_value(value: &Value) -> Option<String> {
    if let Some(obj) = value.as_object() {
        if let Some(err) = obj.get("error_string").and_then(Value::as_str) {
            if !err.is_empty() {
                return Some(err.to_string());
            }
        }
        if let Some(result) = obj.get("result") {
            if !result.is_null() {
                return Some(result.to_string());
            }
        }
        if obj.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }
    if value.is_null() {
        None
    } else {
        Some(value.to_string())
    }
}

fn response_parse_error(msg: &str) -> TransmissionError {
    TransmissionError::Parse(serde_json::Error::io(io::Error::new(
        io::ErrorKind::Other,
        msg.to_string(),
    )))
}

fn method_for_protocol(method: &'static str, protocol: RpcProtocol) -> Cow<'static, str> {
    if matches!(protocol, RpcProtocol::Legacy) {
        Cow::Borrowed(match method {
            "session_get" => "session-get",
            "session_set" => "session-set",
            "session_stats" => "session-stats",
            "torrent_get" => "torrent-get",
            "torrent_add" => "torrent-add",
            "torrent_remove" => "torrent-remove",
            "torrent_start" => "torrent-start",
            "torrent_stop" => "torrent-stop",
            other => other,
        })
    } else {
        Cow::Borrowed(method)
    }
}

fn translate_arguments_for_protocol(
    protocol: RpcProtocol,
    method: &'static str,
    arguments: Option<Value>,
) -> Option<Value> {
    if !matches!(protocol, RpcProtocol::Legacy) {
        return arguments;
    }
    arguments.map(|value| match method {
        "session_get" => map_fields_argument(value, legacy_session_field_name),
        "torrent_get" => map_fields_argument(value, legacy_torrent_field_name),
        "session_set" => map_object_keys(value, legacy_session_field_name),
        "torrent_remove" => rename_key(value, "delete_local_data", "delete-local-data"),
        _ => value,
    })
}

fn map_fields_argument(value: Value, mapper: fn(&str) -> Cow<'static, str>) -> Value {
    if let Value::Object(mut map) = value {
        if let Some(Value::Array(fields)) = map.get_mut("fields") {
            for field in fields.iter_mut() {
                if let Value::String(name) = field {
                    *name = mapper(name).into_owned();
                }
            }
        }
        Value::Object(map)
    } else {
        value
    }
}

fn map_object_keys(value: Value, mapper: fn(&str) -> Cow<'static, str>) -> Value {
    if let Value::Object(map) = value {
        let mut remapped = Map::new();
        for (key, val) in map {
            remapped.insert(mapper(&key).into_owned(), val);
        }
        Value::Object(remapped)
    } else {
        value
    }
}

fn rename_key(value: Value, from: &str, to: &str) -> Value {
    if let Value::Object(mut map) = value {
        if let Some(val) = map.remove(from) {
            map.insert(to.to_string(), val);
        }
        Value::Object(map)
    } else {
        value
    }
}

fn legacy_session_field_name(field: &str) -> Cow<'static, str> {
    match field {
        "download_dir" => Cow::Borrowed("download-dir"),
        "start_added_torrents" => Cow::Borrowed("start-added-torrents"),
        "speed_limit_up" => Cow::Borrowed("speed-limit-up"),
        "speed_limit_up_enabled" => Cow::Borrowed("speed-limit-up-enabled"),
        "speed_limit_down" => Cow::Borrowed("speed-limit-down"),
        "speed_limit_down_enabled" => Cow::Borrowed("speed-limit-down-enabled"),
        "seed_ratio_limited" => Cow::Borrowed("seedRatioLimited"),
        "seed_ratio_limit" => Cow::Borrowed("seedRatioLimit"),
        "idle_seeding_limit_enabled" => Cow::Borrowed("idle-seeding-limit-enabled"),
        "idle_seeding_limit" => Cow::Borrowed("idle-seeding-limit"),
        "peer_limit_per_torrent" => Cow::Borrowed("peer-limit-per-torrent"),
        "peer_limit_global" => Cow::Borrowed("peer-limit-global"),
        "pex_enabled" => Cow::Borrowed("pex-enabled"),
        "dht_enabled" => Cow::Borrowed("dht-enabled"),
        "lpd_enabled" => Cow::Borrowed("lpd-enabled"),
        "blocklist_enabled" => Cow::Borrowed("blocklist-enabled"),
        "blocklist_url" => Cow::Borrowed("blocklist-url"),
        other => Cow::Owned(other.to_string()),
    }
}

fn legacy_torrent_field_name(field: &str) -> Cow<'static, str> {
    match field {
        "percent_done" => Cow::Borrowed("percentDone"),
        "rate_download" => Cow::Borrowed("rateDownload"),
        "rate_upload" => Cow::Borrowed("rateUpload"),
        "upload_ratio" => Cow::Borrowed("uploadRatio"),
        "size_when_done" => Cow::Borrowed("sizeWhenDone"),
        "left_until_done" => Cow::Borrowed("leftUntilDone"),
        "download_dir" => Cow::Borrowed("downloadDir"),
        "peers_connected" => Cow::Borrowed("peersConnected"),
        "peers_sending_to_us" => Cow::Borrowed("peersSendingToUs"),
        "peers_getting_from_us" => Cow::Borrowed("peersGettingFromUs"),
        "error_string" => Cow::Borrowed("errorString"),
        other => Cow::Owned(other.to_string()),
    }
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
    #[serde(default)]
    peers: Vec<PeerWire>,
}

impl From<TorrentWire> for TorrentSummary {
    fn from(wire: TorrentWire) -> Self {
        let TorrentWire {
            id,
            name,
            status,
            percent_done,
            rate_download,
            rate_upload,
            eta,
            upload_ratio,
            size_when_done,
            left_until_done,
            download_dir,
            peers_connected,
            peers_sending_to_us,
            peers_getting_from_us,
            error_string,
            peers,
        } = wire;
        let eta = if eta >= 0 { Some(eta) } else { None };
        let status = match status {
            0 => "stopped".to_string(),
            1 => "check-wait".to_string(),
            2 => "checking".to_string(),
            3 => "download-wait".to_string(),
            4 => "downloading".to_string(),
            5 => "seed-wait".to_string(),
            6 => "seeding".to_string(),
            other => format!("status-{}", other),
        };
        TorrentSummary {
            torrent_id: id,
            name,
            status,
            percent_done,
            rate_download,
            rate_upload,
            eta,
            upload_ratio,
            size_when_done,
            left_until_done,
            download_dir,
            peers_connected,
            peers_sending: peers_sending_to_us,
            peers_receiving: peers_getting_from_us,
            error: if error_string.is_empty() {
                None
            } else {
                Some(error_string)
            },
            peers: peers.into_iter().map(PeerSummary::from).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PeerWire {
    #[serde(default)]
    address: String,
    #[serde(default, alias = "clientName")]
    client_name: String,
    #[serde(default)]
    progress: f64,
    #[serde(default, alias = "rateToClient")]
    rate_to_client: i64,
    #[serde(default, alias = "rateToPeer")]
    rate_to_peer: i64,
}

impl From<PeerWire> for PeerSummary {
    fn from(wire: PeerWire) -> Self {
        Self {
            address: wire.address,
            client: wire.client_name,
            progress: wire.progress,
            rate_down: wire.rate_to_client,
            rate_up: wire.rate_to_peer,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AddTorrentResponse {
    #[serde(
        rename = "torrent_added",
        alias = "torrent-added",
        alias = "torrentAdded"
    )]
    torrent_added: Option<TorrentRef>,
    #[serde(
        rename = "torrent_duplicate",
        alias = "torrent-duplicate",
        alias = "torrentDuplicate"
    )]
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
