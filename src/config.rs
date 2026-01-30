use std::{
    env, fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::{ArgAction, Parser};
use dirs::config_dir;
use log::LevelFilter;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub rpc: RpcConfig,
    pub poll_interval: Duration,
    pub log_level: LevelFilter,
}

#[derive(Debug, Clone)]
pub struct RpcConfig {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub timeout: Duration,
    pub verify_ssl: bool,
    pub user_agent: String,
    pub url: Option<String>,
}

impl RpcConfig {
    pub fn endpoint(&self) -> String {
        if let Some(url) = &self.url {
            return url.clone();
        }
        let mut path = self.path.clone();
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        format!("{}://{}:{}{}", self.scheme, self.host, self.port, path)
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about = "Transmission daemon terminal UI", long_about = None)]
pub struct Cli {
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub path: Option<String>,
    #[arg(long)]
    pub username: Option<String>,
    #[arg(long)]
    pub password: Option<String>,
    #[arg(long)]
    pub timeout: Option<f64>,
    #[arg(long)]
    pub poll_interval: Option<f64>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub tls: bool,
    #[arg(long = "no-tls", action = ArgAction::SetTrue)]
    pub no_tls: bool,
    #[arg(long)]
    pub insecure: bool,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub log_level: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    rpc: Option<FileRpcConfig>,
    poll_interval: Option<f64>,
    log_level: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileRpcConfig {
    url: Option<String>,
    scheme: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    path: Option<String>,
    username: Option<String>,
    password: Option<String>,
    timeout: Option<f64>,
    tls: Option<bool>,
    verify_ssl: Option<bool>,
    user_agent: Option<String>,
}

pub fn build_config(cli: &Cli) -> Result<AppConfig> {
    let file_config = load_file_config(cli.config.as_deref())?;
    let rpc_file = file_config.as_ref().and_then(|cfg| cfg.rpc.as_ref());

    let url = cli
        .url
        .clone()
        .or_else(|| env::var("TRANSMISSION_URL").ok())
        .or_else(|| rpc_file.and_then(|cfg| cfg.url.clone()));

    let host = cli
        .host
        .clone()
        .or_else(|| env::var("TRANSMISSION_HOST").ok())
        .or_else(|| rpc_file.and_then(|cfg| cfg.host.clone()))
        .unwrap_or_else(|| "localhost".to_string());

    let port = cli
        .port
        .or_else(|| env_var_parse("TRANSMISSION_PORT"))
        .or_else(|| rpc_file.and_then(|cfg| cfg.port))
        .unwrap_or(9091);

    let path = cli
        .path
        .clone()
        .or_else(|| env::var("TRANSMISSION_RPC_PATH").ok())
        .or_else(|| rpc_file.and_then(|cfg| cfg.path.clone()))
        .unwrap_or_else(|| "/transmission/rpc".to_string());

    let username = cli
        .username
        .clone()
        .or_else(|| env::var("TRANSMISSION_USERNAME").ok())
        .or_else(|| rpc_file.and_then(|cfg| cfg.username.clone()));

    let password = cli
        .password
        .clone()
        .or_else(|| env::var("TRANSMISSION_PASSWORD").ok())
        .or_else(|| rpc_file.and_then(|cfg| cfg.password.clone()));

    let timeout_secs = cli
        .timeout
        .or_else(|| env_float("TRANSMISSION_TIMEOUT"))
        .or_else(|| rpc_file.and_then(|cfg| cfg.timeout))
        .unwrap_or(10.0);

    if timeout_secs <= 0.0 {
        anyhow::bail!("timeout must be positive");
    }

    let poll_secs = cli
        .poll_interval
        .or_else(|| env_float("TRANSMISSION_POLL_INTERVAL"))
        .or_else(|| file_config.as_ref().and_then(|cfg| cfg.poll_interval))
        .unwrap_or(3.0);

    if poll_secs < 0.0 {
        anyhow::bail!("poll interval cannot be negative");
    }

    let tls_flag = if cli.tls {
        Some(true)
    } else if cli.no_tls {
        Some(false)
    } else {
        None
    };

    let tls_env = env_bool("TRANSMISSION_TLS");
    let use_tls = tls_flag
        .or(tls_env)
        .or_else(|| rpc_file.and_then(|cfg| cfg.tls))
        .unwrap_or(false);

    let verify_env = env_bool("TRANSMISSION_VERIFY_SSL");
    let mut verify_ssl = rpc_file.and_then(|cfg| cfg.verify_ssl).unwrap_or(true);
    if let Some(value) = verify_env {
        verify_ssl = value;
    }
    if cli.insecure {
        verify_ssl = false;
    }

    let scheme = rpc_file
        .and_then(|cfg| cfg.scheme.clone())
        .unwrap_or_else(|| if use_tls { "https" } else { "http" }.to_string());

    let user_agent = env::var("TRANSMISSION_USER_AGENT")
        .ok()
        .or_else(|| rpc_file.and_then(|cfg| cfg.user_agent.clone()))
        .unwrap_or_else(|| "transmission-tui".to_string());

    let log_level_str = cli
        .log_level
        .clone()
        .or_else(|| env::var("TRANSMISSION_LOG_LEVEL").ok())
        .or_else(|| file_config.as_ref().and_then(|cfg| cfg.log_level.clone()))
        .unwrap_or_else(|| "info".to_string());
    let log_level = LevelFilter::from_str(&log_level_str).unwrap_or(LevelFilter::Info);

    Ok(AppConfig {
        rpc: RpcConfig {
            scheme,
            host,
            port,
            path,
            username,
            password,
            timeout: Duration::from_secs_f64(timeout_secs),
            verify_ssl,
            user_agent,
            url,
        },
        poll_interval: Duration::from_secs_f64(poll_secs.max(0.0)),
        log_level,
    })
}

fn load_file_config(path: Option<&Path>) -> Result<Option<FileConfig>> {
    if let Some(path) = path {
        return read_file_config(path);
    }

    if let Ok(env_path) = env::var("TRANSMISSION_TUI_CONFIG") {
        return read_file_config(Path::new(&env_path));
    }

    if let Some(dir) = config_dir() {
        let modern_path = dir.join("transmission-tui").join("config.toml");
        if let Some(cfg) = read_file_config(&modern_path)? {
            return Ok(Some(cfg));
        }

        let legacy_path = dir.join("transmission-tui.toml");
        return read_file_config(&legacy_path);
    }

    Ok(None)
}

fn read_file_config(path: &Path) -> Result<Option<FileConfig>> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let parsed: FileConfig = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(Some(parsed))
}

fn env_var_parse<T>(name: &str) -> Option<T>
where
    T: FromStr,
{
    env::var(name).ok().and_then(|value| value.parse().ok())
}

fn env_float(name: &str) -> Option<f64> {
    env_var_parse(name)
}

fn env_bool(name: &str) -> Option<bool> {
    env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}
