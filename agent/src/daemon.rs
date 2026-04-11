use std::{env, fs, path::{Path as FsPath, PathBuf}, sync::Arc};

use anyhow::{anyhow, Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
    routing::{delete, get, post},
};
use chrono::{SecondsFormat, Utc};
use did_ma::{Did, Document, EncryptionKey, SigningKey, VerificationMethod};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey, endpoint::presets};
use ma_core::{CONTENT_TYPE_WORLD, DEFAULT_WORLD_RELAY_URL, INBOX_ALPN, MessageEnvelope, WorldCommand, WorldRequest, WorldResponse, default_ma_config_root, normalize_relay_url, parse_message, resolve_inbox_endpoint_id};
use rand::RngCore;
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};

const AGENT_VERSION_FILE: &str = ".generated/agent-version.txt";
const AGENT_IROH_KEY_FILE: &str = "agent_iroh.bin";
const AGENT_ENC_KEY_FILE: &str = "agent_enc.privkey";
const AGENT_SIG_KEY_FILE: &str = "agent_sig.privkey";
const ADMIN_DASHBOARD_HTML: &str = include_str!("../www/index.html");

#[derive(Debug, Clone)]
struct DaemonSecretPaths {
    iroh_path: PathBuf,
    enc_path: PathBuf,
    sig_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentdConfig {
    pub listen: String,
    pub kubo_api_url: String,
    pub kubo_key_alias: String,
    pub passphrase: String,
    pub world_key_file: String,
    pub iroh_key_file: String,
    pub enc_key_file: String,
    pub sig_key_file: String,
    pub world_did_root: String,
    #[serde(default, alias = "poll_interval")]
    pub poll_ttl: u64,
    #[serde(default)]
    pub lock_ttl: u64,
}

impl Default for AgentdConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:5003".to_string(),
            kubo_api_url: "http://127.0.0.1:5001".to_string(),
            kubo_key_alias: "ma-agent".to_string(),
            passphrase: "dev-passphrase".to_string(),
            world_key_file: "agentd_world.key".to_string(),
            iroh_key_file: AGENT_IROH_KEY_FILE.to_string(),
            enc_key_file: AGENT_ENC_KEY_FILE.to_string(),
            sig_key_file: AGENT_SIG_KEY_FILE.to_string(),
            world_did_root: String::new(),
            poll_ttl: 10,
            lock_ttl: 120,
        }
    }
}

impl AgentdConfig {
    fn normalize(&mut self) {
        self.listen = self.listen.trim().to_string();
        if self.listen.is_empty() {
            self.listen = "127.0.0.1:5003".to_string();
        }

        self.kubo_api_url = self.kubo_api_url.trim().to_string();
        if self.kubo_api_url.is_empty() {
            self.kubo_api_url = "http://127.0.0.1:5001".to_string();
        }

        self.kubo_key_alias = self.kubo_key_alias.trim().to_string();
        if self.kubo_key_alias.is_empty() {
            self.kubo_key_alias = "ma-agent".to_string();
        } else if self.kubo_key_alias == "agentd" {
            // Migrate legacy default alias to the canonical ma-agent alias.
            self.kubo_key_alias = "ma-agent".to_string();
        }

        self.passphrase = self.passphrase.trim().to_string();
        if self.passphrase.is_empty() {
            self.passphrase = "dev-passphrase".to_string();
        }

        self.world_key_file = self.world_key_file.trim().to_string();
        if self.world_key_file.is_empty() {
            self.world_key_file = "agentd_world.key".to_string();
        }

        self.iroh_key_file = self.iroh_key_file.trim().to_string();
        if self.iroh_key_file.is_empty() {
            self.iroh_key_file = AGENT_IROH_KEY_FILE.to_string();
        }

        self.enc_key_file = self.enc_key_file.trim().to_string();
        if self.enc_key_file.is_empty() {
            self.enc_key_file = AGENT_ENC_KEY_FILE.to_string();
        }

        self.sig_key_file = self.sig_key_file.trim().to_string();
        if self.sig_key_file.is_empty() {
            self.sig_key_file = AGENT_SIG_KEY_FILE.to_string();
        }

        self.world_did_root = self.world_did_root.trim().to_string();
        self.poll_ttl = if self.poll_ttl == 0 { 10 } else { self.poll_ttl };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentMeta {
    id: String,
    did: String,
    signing_private_key_hex: Option<String>,
    encryption_private_key_hex: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Clone)]
struct AppState {
    cfg: Arc<RwLock<AgentdConfig>>,
    config_path: PathBuf,
    data_root: PathBuf,
    agents_dir: PathBuf,
    logs_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CreateAgentRequest {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    q: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AppendLogRequest {
    line: String,
}

#[derive(Debug, Deserialize)]
struct SendAgentRequest {
    room: Option<String>,
    to: Option<String>,
    mode: Option<String>,
    body: String,
    ttl_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UpdateConfigRequest {
    kubo_key_alias: Option<String>,
    lock_ttl: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ValidateAliasQuery {
    alias: Option<String>,
}

#[derive(Debug, Serialize)]
struct GenericResponse {
    ok: bool,
    message: String,
}

#[derive(Debug, Serialize)]
struct SendAgentResponse {
    ok: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    world_response: Option<WorldResponse>,
}

#[derive(Debug, Serialize)]
struct CreateAgentResponse {
    ok: bool,
    message: String,
    agent: Option<AgentInfoResponse>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    agent_version: String,
    listen: String,
    kubo_api_url: String,
    kubo_key_alias: String,
    kubo_key_alias_found: bool,
    world_did_root: String,
    world_key_file: String,
    iroh_key_file: String,
    enc_key_file: String,
    sig_key_file: String,
    config_path: String,
    data_root: String,
}

#[derive(Debug, Serialize)]
struct AgentInfoResponse {
    id: String,
    did: String,
    created_at: String,
    updated_at: String,
    log_path: String,
}

#[derive(Debug, Serialize)]
struct ListAgentsResponse {
    ok: bool,
    agents: Vec<AgentInfoResponse>,
}

#[derive(Debug, Serialize)]
struct LogsResponse {
    ok: bool,
    agent_id: String,
    lines: Vec<String>,
    total_lines_scanned: usize,
}

#[derive(Debug, Serialize)]
struct ConfigResponse {
    ok: bool,
    message: String,
    config: AgentdConfig,
    config_path: String,
    restart_required: bool,
}

#[derive(Debug, Serialize)]
struct ValidateAliasResponse {
    ok: bool,
    alias: String,
    found: bool,
    message: String,
}

pub async fn run_daemon(
    listen_override: Option<String>,
    kubo_key_alias_override: Option<String>,
    config_path_override: Option<PathBuf>,
) -> Result<()> {
    let (cfg, config_path) = load_or_init_config(
        listen_override,
        kubo_key_alias_override,
        config_path_override.clone(),
    )?;
    let state = build_state(cfg, config_path)?;

    let app = Router::new()
        .route("/", get(index))
        .route("/api/v0/health", get(health))
        .route("/api/v0/config", get(get_config))
        .route("/api/v0/config/update", post(update_config))
        .route("/api/v0/config/validate-key-alias", get(validate_key_alias))
        .route("/api/v0/agents", get(list_agents))
        .route("/api/v0/agents/create", post(create_agent))
        .route("/api/v0/agents/{id}", delete(delete_agent))
        .route("/api/v0/agents/{id}/logs", get(get_logs))
        .route("/api/v0/agents/{id}/log", post(append_log))
        .route("/api/v0/agents/{id}/send", post(send_agent))
        .with_state(state.clone());

    let cfg_now = state.cfg.read().await.clone();

    let listener = TcpListener::bind(&cfg_now.listen)
        .await
        .with_context(|| format!("failed to bind {}", cfg_now.listen))?;

    println!("ma-agentd listening on http://{}", cfg_now.listen);
    println!("config: {}", state.config_path.display());
    println!("data:   {}", state.data_root.display());
    println!("alias monitor: every {}s", cfg_now.poll_ttl);
    if let Ok(paths) = ensure_daemon_secret_files_for(&cfg_now, &state.config_path) {
        println!("keys:   iroh={} enc={} sig={}", paths.iroh_path.display(), paths.enc_path.display(), paths.sig_path.display());
    }

    let startup_state = state.clone();
    tokio::spawn(async move {
        match ensure_world_root_did_published(&startup_state).await {
            Ok(root) => println!("startup publish: world DID ready at {}", root),
            Err(err) => eprintln!("startup publish: WARN could not ensure world DID: {}", err),
        }
    });

    tokio::spawn(alias_monitor_loop(state.clone()));
    tokio::spawn(sighup_reload_loop(state.clone()));

    axum::serve(listener, app).await?;
    Ok(())
}

async fn alias_monitor_loop(state: AppState) {
    loop {
        let cfg = state.cfg.read().await.clone();
        if !cfg.world_did_root.trim().is_empty() {
            println!("alias monitor: world DID root present; stopping Kubo alias polling");
            return;
        }

        let alias = kubo_key_alias(&cfg);
        match kubo_list_keys(&cfg.kubo_api_url).await {
            Ok(keys) => {
                if keys.iter().any(|k| k.name == alias) {
                    println!("alias monitor: OK alias '{}' found in Kubo", alias);
                } else {
                    eprintln!(
                        "alias monitor: WARN alias '{}' not found in Kubo (waiting for manual setup)",
                        alias
                    );
                }
            }
            Err(err) => {
                eprintln!(
                    "alias monitor: WARN failed listing Kubo keys for alias '{}': {}",
                    alias, err
                );
            }
        }

        sleep(Duration::from_secs(cfg.poll_ttl)).await;
    }
}

#[cfg(unix)]
async fn sighup_reload_loop(state: AppState) {
    use tokio::signal::unix::{SignalKind, signal};

    let Ok(mut stream) = signal(SignalKind::hangup()) else {
        eprintln!("WARN: failed to subscribe to SIGHUP");
        return;
    };

    while stream.recv().await.is_some() {
        match reload_runtime_config(&state).await {
            Ok(cfg) => println!(
                "SIGHUP: reloaded config (alias='{}', poll_ttl={}s)",
                cfg.kubo_key_alias,
                cfg.poll_ttl
            ),
            Err(err) => eprintln!("WARN: SIGHUP reload failed: {}", err),
        }
    }
}

#[cfg(not(unix))]
async fn sighup_reload_loop(_state: AppState) {}

fn load_or_init_config(
    listen_override: Option<String>,
    kubo_key_alias_override: Option<String>,
    config_path_override: Option<PathBuf>,
) -> Result<(AgentdConfig, PathBuf)> {
    let config_path = if let Some(path) = config_path_override {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir {}", parent.display()))?;
        }
        path
    } else {
        daemon_config_root()?.join("agentd.yaml")
    };

    let mut cfg = if config_path.exists() {
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        serde_yaml::from_str::<AgentdConfig>(&raw)
            .with_context(|| format!("failed to parse {}", config_path.display()))?
    } else {
        AgentdConfig {
            listen: "127.0.0.1:5003".to_string(),
            kubo_api_url: "http://127.0.0.1:5001".to_string(),
            kubo_key_alias: "ma-agent".to_string(),
            poll_ttl: 10,
            lock_ttl: 120,
            passphrase: "dev-passphrase".to_string(),
            world_key_file: "agentd_world.key".to_string(),
            iroh_key_file: AGENT_IROH_KEY_FILE.to_string(),
            enc_key_file: AGENT_ENC_KEY_FILE.to_string(),
            sig_key_file: AGENT_SIG_KEY_FILE.to_string(),
            world_did_root: String::new(),
        }
    };

    if let Some(override_listen) = listen_override {
        let value = override_listen.trim();
        if !value.is_empty() {
            cfg.listen = value.to_string();
        }
    }

    if let Some(override_alias) = kubo_key_alias_override {
        let value = override_alias.trim();
        if !value.is_empty() {
            cfg.kubo_key_alias = value.to_string();
        }
    }

    cfg.normalize();
    let _ = ensure_daemon_secret_files_for(&cfg, &config_path)?;

    let yaml = serde_yaml::to_string(&cfg)?;
    fs::write(&config_path, yaml)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok((cfg, config_path))
}

fn build_state(cfg: AgentdConfig, config_path: PathBuf) -> Result<AppState> {
    let data_root = data_root_global()?;
    let agents_dir = data_root.join("agents");
    let logs_dir = data_root.join("logs");

    fs::create_dir_all(&agents_dir)
        .with_context(|| format!("failed to create {}", agents_dir.display()))?;
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    migrate_legacy_agent_data(&data_root, &agents_dir, &logs_dir)?;

    Ok(AppState {
        cfg: Arc::new(RwLock::new(cfg)),
        config_path,
        data_root,
        agents_dir,
        logs_dir,
    })
}

fn daemon_config_root() -> Result<PathBuf> {
    if let Ok(xdg_root) = env::var("XDG_CONFIG_HOME") {
        let root = PathBuf::from(xdg_root).join("ma");
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create config dir {}", root.display()))?;
        return Ok(root);
    }
    let root = default_ma_config_root()?;
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create config dir {}", root.display()))?;
    Ok(root)
}

fn daemon_secret_paths_for_cfg(config_root: &FsPath, cfg: &AgentdConfig) -> DaemonSecretPaths {
    DaemonSecretPaths {
        iroh_path: config_root.join(cfg.iroh_key_file.trim()),
        enc_path: config_root.join(cfg.enc_key_file.trim()),
        sig_path: config_root.join(cfg.sig_key_file.trim()),
    }
}

fn ensure_secret_file(path: &FsPath, expected_len: usize, label: &str) -> Result<Vec<u8>> {
    if path.exists() {
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        if bytes.len() != expected_len {
            return Err(anyhow!(
                "invalid {} length in {} (expected {} bytes, got {})",
                label,
                path.display(),
                expected_len,
                bytes.len()
            ));
        }
        return Ok(bytes);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create key dir {}", parent.display()))?;
    }

    let mut bytes = vec![0u8; expected_len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    fs::write(path, &bytes).with_context(|| format!("failed to write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed setting permissions on {}", path.display()))?;
    }

    Ok(bytes)
}

fn config_root_from_config_path(config_path: &FsPath) -> Result<PathBuf> {
    let Some(parent) = config_path.parent() else {
        return Err(anyhow!("config path has no parent: {}", config_path.display()));
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create config dir {}", parent.display()))?;
    Ok(parent.to_path_buf())
}

fn ensure_daemon_secret_files_for(cfg: &AgentdConfig, config_path: &FsPath) -> Result<DaemonSecretPaths> {
    let config_root = config_root_from_config_path(config_path)?;
    let paths = daemon_secret_paths_for_cfg(&config_root, cfg);
    ensure_secret_file(&paths.iroh_path, 32, "agent iroh secret")?;
    ensure_secret_file(&paths.enc_path, 32, "agent encryption secret")?;
    ensure_secret_file(&paths.sig_path, 32, "agent signing secret")?;
    Ok(paths)
}

async fn reload_runtime_config(state: &AppState) -> Result<AgentdConfig> {
    let mut cfg = load_or_default_config()?;
    cfg.normalize();
    let _ = ensure_daemon_secret_files_for(&cfg, &state.config_path)?;
    {
        let mut guard = state.cfg.write().await;
        *guard = cfg.clone();
    }
    Ok(cfg)
}

async fn persist_runtime_config(state: &AppState, cfg: AgentdConfig) -> Result<PathBuf> {
    let _ = ensure_daemon_secret_files_for(&cfg, &state.config_path)?;
    let path = write_config_to_path(&state.config_path, &cfg)?;
    {
        let mut guard = state.cfg.write().await;
        *guard = cfg;
    }
    Ok(path)
}

fn load_or_default_config() -> Result<AgentdConfig> {
    let config_root = daemon_config_root()?;
    let config_path = config_root.join("agentd.yaml");

    if config_path.exists() {
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let mut cfg = serde_yaml::from_str::<AgentdConfig>(&raw)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
        cfg.normalize();
        let _ = ensure_daemon_secret_files_for(&cfg, &config_path)?;
        return Ok(cfg);
    }

    let mut cfg = AgentdConfig {
        listen: "127.0.0.1:5003".to_string(),
        kubo_api_url: "http://127.0.0.1:5001".to_string(),
        kubo_key_alias: "ma-agent".to_string(),
        poll_ttl: 10,
        lock_ttl: 120,
        passphrase: "dev-passphrase".to_string(),
        world_key_file: "agentd_world.key".to_string(),
        iroh_key_file: AGENT_IROH_KEY_FILE.to_string(),
        enc_key_file: AGENT_ENC_KEY_FILE.to_string(),
        sig_key_file: AGENT_SIG_KEY_FILE.to_string(),
        world_did_root: String::new(),
    };
    cfg.normalize();
    Ok(cfg)
}

fn write_config_to_path(config_path: &FsPath, cfg: &AgentdConfig) -> Result<PathBuf> {
    let _ = config_root_from_config_path(config_path)?;
    let yaml = serde_yaml::to_string(cfg)?;
    fs::write(&config_path, yaml)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(config_path.to_path_buf())
}

fn read_agent_version() -> Option<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(AGENT_VERSION_FILE);
    let raw = fs::read_to_string(path).ok()?;
    let value = raw.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn data_root_base() -> Result<PathBuf> {
    let base = if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg_data)
    } else {
        let home = env::var("HOME").context("HOME is not set")?;
        PathBuf::from(home).join(".local").join("share")
    };
    Ok(base.join("ma"))
}

fn data_root_global() -> Result<PathBuf> {
    Ok(data_root_base()?.join("agentd"))
}

fn migrate_legacy_agent_data(data_root: &FsPath, agents_dir: &FsPath, logs_dir: &FsPath) -> Result<()> {
    let ma_root = data_root
        .parent()
        .ok_or_else(|| anyhow!("invalid data root {}", data_root.display()))?;

    let Ok(entries) = fs::read_dir(ma_root) else {
        return Ok(());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if name == "agentd" {
            continue;
        }

        let legacy_agents = path.join("agents");
        let legacy_logs = path.join("logs");

        if let Ok(files) = fs::read_dir(&legacy_agents) {
            for file in files.flatten() {
                let src = file.path();
                if src.extension().and_then(|v| v.to_str()) != Some("json") {
                    continue;
                }
                let Some(file_name) = src.file_name() else {
                    continue;
                };
                let dst = agents_dir.join(file_name);
                if !dst.exists() {
                    let _ = fs::copy(&src, &dst);
                }
            }
        }

        if let Ok(files) = fs::read_dir(&legacy_logs) {
            for file in files.flatten() {
                let src = file.path();
                if src.extension().and_then(|v| v.to_str()) != Some("log") {
                    continue;
                }
                let Some(file_name) = src.file_name() else {
                    continue;
                };
                let dst = logs_dir.join(file_name);
                if !dst.exists() {
                    let _ = fs::copy(&src, &dst);
                }
            }
        }
    }

    Ok(())
}

fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn generate_agent_id() -> String {
    nanoid::nanoid!(12)
}

fn agent_meta_path(state: &AppState, agent_id: &str) -> PathBuf {
    state.agents_dir.join(format!("{}.json", agent_id))
}

fn agent_log_path(state: &AppState, agent_id: &str) -> PathBuf {
    state.logs_dir.join(format!("{}.log", agent_id))
}

fn append_agent_log_line(state: &AppState, agent_id: &str, line: &str) -> Result<()> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("line must not be empty"));
    }

    let log_path = agent_log_path(state, agent_id);
    let stamped = format!("{} {}\n", Utc::now().to_rfc3339(), trimmed);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, stamped.as_bytes()))
        .with_context(|| format!("failed appending log '{}'", log_path.display()))?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct KuboKeyListResponse {
    #[serde(rename = "Keys")]
    keys_upper: Option<Vec<KuboKeyItem>>,
    #[serde(rename = "keys")]
    keys_lower: Option<Vec<KuboKeyItem>>,
}

#[derive(Debug, Deserialize)]
struct KuboKeyItem {
    #[serde(rename = "Name")]
    name_upper: Option<String>,
    #[serde(rename = "Id")]
    id_upper: Option<String>,
    #[serde(rename = "name")]
    name_lower: Option<String>,
    #[serde(rename = "id")]
    id_lower: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IpfsAddResponse {
    #[serde(rename = "Hash")]
    hash_upper: Option<String>,
    #[serde(rename = "hash")]
    hash_lower: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NameResolveResponse {
    #[serde(rename = "Path")]
    path_upper: Option<String>,
    #[serde(rename = "path")]
    path_lower: Option<String>,
}

#[derive(Debug, Clone)]
struct KuboKeyInfo {
    name: String,
    id: String,
}

fn kubo_key_alias(cfg: &AgentdConfig) -> String {
    let alias = cfg.kubo_key_alias.trim();
    if alias.is_empty() {
        "ma-agent".to_string()
    } else {
        alias.to_string()
    }
}

async fn kubo_create_key(kubo_url: &str, key_name: &str) -> Result<()> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/key/gen");

    reqwest::Client::new()
        .post(url)
        .query(&[("arg", key_name), ("type", "ed25519")])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn kubo_list_keys(kubo_url: &str) -> Result<Vec<KuboKeyInfo>> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/key/list");
    let body = reqwest::Client::new()
        .post(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let parsed: KuboKeyListResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing key/list response: {} body={}", e, body))?;

    let items = parsed.keys_upper.or(parsed.keys_lower).unwrap_or_default();
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let name = item
                .name_upper
                .or(item.name_lower)
                .unwrap_or_default()
                .trim()
                .to_string();
            let id = item
                .id_upper
                .or(item.id_lower)
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() || id.is_empty() {
                None
            } else {
                Some(KuboKeyInfo { name, id })
            }
        })
        .collect())
}

async fn kubo_ipfs_add(kubo_url: &str, data: Vec<u8>) -> Result<String> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/add");
    let part = multipart::Part::bytes(data)
        .file_name("did.json")
        .mime_str("application/json")?;
    let form = multipart::Form::new().part("file", part);

    let body = reqwest::Client::new()
        .post(url)
        .query(&[("pin", "true"), ("cid-version", "1")])
        .multipart(form)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let parsed: IpfsAddResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing add response: {} body={}", e, body))?;
    let cid = parsed.hash_upper.or(parsed.hash_lower).unwrap_or_default();
    if cid.trim().is_empty() {
        return Err(anyhow!("missing hash in add response: {}", body));
    }
    Ok(cid)
}

async fn kubo_ipns_publish(kubo_url: &str, key_name: &str, cid: &str) -> Result<()> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/name/publish");
    let arg = format!("/ipfs/{}", cid.trim().trim_start_matches("/ipfs/"));

    reqwest::Client::new()
        .post(url)
        .query(&[("arg", arg.as_str()), ("key", key_name), ("allow-offline", "true")])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn kubo_name_resolve(kubo_url: &str, path: &str) -> Result<String> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/name/resolve");
    let body = reqwest::Client::new()
        .post(url)
        .query(&[("arg", path), ("recursive", "true")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let parsed: NameResolveResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing name/resolve response: {} body={}", e, body))?;
    let resolved = parsed.path_upper.or(parsed.path_lower).unwrap_or_default();
    if resolved.trim().is_empty() {
        return Err(anyhow!("missing path in name/resolve response: {}", body));
    }
    Ok(resolved)
}

async fn kubo_cat_text(kubo_url: &str, path: &str) -> Result<String> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/cat");
    let body = reqwest::Client::new()
        .post(url)
        .query(&[("arg", path)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

async fn resolved_root_did_document(kubo_url: &str, resolved_path: &str, root_did: &str) -> Result<Option<Document>> {
    let path = resolved_path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let raw = kubo_cat_text(kubo_url, path).await?;
    let doc = Document::unmarshal(&raw)
        .map_err(|e| anyhow!("failed to decode DID document at {}: {}", path, e))?;
    if doc.id == root_did {
        Ok(Some(doc))
    } else {
        Ok(None)
    }
}

fn expected_agent_transports(endpoint_id: &str) -> serde_json::Value {
    let lanes = [String::from_utf8_lossy(INBOX_ALPN).to_string()];
    serde_json::Value::Array(
        lanes
            .into_iter()
            .map(|lane| serde_json::Value::String(format!("/ma-iroh/{}/{}", endpoint_id, lane)))
            .collect(),
    )
}

fn now_zulu() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn iroh_endpoint_id_from_secret_file(path: &FsPath) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let key_bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid iroh secret key length in {}", path.display()))?;
    let secret = SecretKey::from_bytes(&key_bytes);
    let endpoint_id = EndpointId::from(secret.public());
    Ok(endpoint_id.to_string())
}

fn did_doc_has_required_ma_fields(doc: &Document) -> bool {
    let Some(ma) = doc.ma.as_ref() else {
        return false;
    };

    let has_transports = ma
        .transports
        .as_ref()
        .and_then(|value| value.as_array())
        .map(|entries| !entries.is_empty())
        .unwrap_or(false);
    let has_created = ma.created.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false);
    let has_updated = ma.updated.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false);

    has_transports && has_created && has_updated
}

async fn ensure_world_root_did_published(state: &AppState) -> Result<String> {
    let cfg = state.cfg.read().await.clone();
    let kubo_url = cfg.kubo_api_url.trim();
    if kubo_url.is_empty() {
        return Err(anyhow!("kubo_api_url is empty"));
    }

    let world_key_name = kubo_key_alias(&cfg);

    let key_id = {
        let mut keys = kubo_list_keys(kubo_url).await?;
        if keys.iter().all(|key| key.name != world_key_name) {
            println!(
                "startup publish: creating missing kubo key alias '{}'",
                world_key_name
            );
            kubo_create_key(kubo_url, &world_key_name).await?;
            keys = kubo_list_keys(kubo_url).await?;
        }

        if let Some(existing) = keys.iter().find(|key| key.name == world_key_name) {
            existing.id.clone()
        } else {
            return Err(anyhow!(
                "kubo key alias '{}' is missing after create attempt",
                world_key_name
            ));
        }
    };

    let root_did = format!("did:ma:{}", key_id);
    let ipns_path = format!("/ipns/{}", key_id);
    let mut existing_created: Option<String> = None;
    if let Ok(resolved) = kubo_name_resolve(kubo_url, &ipns_path).await {
        match resolved_root_did_document(kubo_url, &resolved, &root_did).await {
            Ok(Some(existing)) => {
                if did_doc_has_required_ma_fields(&existing) {
                    if cfg.world_did_root != root_did {
                        let mut next = cfg.clone();
                        next.world_did_root = root_did.clone();
                        persist_runtime_config(state, next).await?;
                    }
                    return Ok(root_did);
                }

                existing_created = existing
                    .ma
                    .as_ref()
                    .and_then(|ma| ma.created.as_ref())
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                eprintln!(
                    "WARN: existing DID document is missing required ma fields (transports/created/updated); republishing"
                );
            }
            Ok(None) => {
                eprintln!(
                    "WARN: IPNS '{}' resolves to '{}', but document id did not match '{}'; republishing",
                    ipns_path, resolved, root_did
                );
            }
            Err(err) => {
                eprintln!(
                    "WARN: resolved IPNS '{}' but could not validate DID doc ({}); republishing",
                    ipns_path, err
                );
            }
        }
    }

    let secret_paths = ensure_daemon_secret_files_for(&cfg, &state.config_path)?;
    let iroh_endpoint_id = iroh_endpoint_id_from_secret_file(&secret_paths.iroh_path)?;
    let signing_bytes = ensure_secret_file(&secret_paths.sig_path, 32, "agent signing secret")?;
    let encryption_bytes = ensure_secret_file(&secret_paths.enc_path, 32, "agent encryption secret")?;

    let root_did_struct = Did::new_root(&key_id)?;
    let signing_did = Did::new(&key_id, "sig")?;
    let encryption_did = Did::new(&key_id, "enc")?;

    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        signing_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid signing key length in {}", secret_paths.sig_path.display()))?,
    )?;
    let encryption_key = EncryptionKey::from_private_key_bytes(
        encryption_did,
        encryption_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid encryption key length in {}", secret_paths.enc_path.display()))?,
    )?;

    let mut document = Document::new(&root_did_struct, &root_did_struct);
    let assertion_vm = VerificationMethod::new(
        root_did_struct.base_id(),
        root_did_struct.base_id(),
        signing_key.key_type.clone(),
        "sig",
        signing_key.public_key_multibase.clone(),
    )?;
    let key_agreement_vm = VerificationMethod::new(
        root_did_struct.base_id(),
        root_did_struct.base_id(),
        encryption_key.key_type.clone(),
        "enc",
        encryption_key.public_key_multibase.clone(),
    )?;
    let assertion_vm_id = assertion_vm.id.clone();
    document.add_verification_method(assertion_vm.clone())?;
    document.add_verification_method(key_agreement_vm.clone())?;
    document.assertion_method = assertion_vm_id;
    document.key_agreement = key_agreement_vm.id.clone();
    document.set_ma_type("agent")?;
    let now = now_zulu();
    document.set_ma_transports(expected_agent_transports(&iroh_endpoint_id));
    document.set_ma_created(existing_created.unwrap_or_else(|| now.clone()));
    document.set_ma_updated(now);
    if let Some(version) = read_agent_version() {
        document.set_ma_version_id(version);
    }
    document.sign(&signing_key, &assertion_vm)?;
    let document_json = document.marshal()
        .map_err(|e| anyhow!("failed to marshal world root DID document: {}", e))?;
    let cid = kubo_ipfs_add(kubo_url, document_json.into_bytes()).await?;
    kubo_ipns_publish(kubo_url, &world_key_name, &cid).await?;

    if cfg.world_did_root != root_did {
        let mut next = cfg.clone();
        next.world_did_root = root_did.clone();
        persist_runtime_config(state, next).await?;
    }

    Ok(root_did)
}

fn load_agent_meta(state: &AppState, agent_id: &str) -> Result<AgentMeta> {
    let path = agent_meta_path(state, agent_id);
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let meta = serde_json::from_str::<AgentMeta>(&raw)
        .with_context(|| format!("invalid agent metadata {}", path.display()))?;
    Ok(meta)
}

fn save_agent_meta(state: &AppState, meta: &AgentMeta) -> Result<()> {
    let path = agent_meta_path(state, &meta.id);
    let raw = serde_json::to_string_pretty(meta)?;
    fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    Html(ADMIN_DASHBOARD_HTML)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let cfg = state.cfg.read().await.clone();
    let paths = ensure_daemon_secret_files_for(&cfg, &state.config_path).ok();
    let alias = kubo_key_alias(&cfg);
    let alias_found = kubo_list_keys(&cfg.kubo_api_url)
        .await
        .map(|keys| keys.iter().any(|k| k.name == alias))
        .unwrap_or(false);

    Json(HealthResponse {
        ok: true,
        agent_version: read_agent_version().unwrap_or_else(|| "dev".to_string()),
        listen: cfg.listen,
        kubo_api_url: cfg.kubo_api_url,
        kubo_key_alias: cfg.kubo_key_alias,
        kubo_key_alias_found: alias_found,
        world_did_root: cfg.world_did_root,
        world_key_file: cfg.world_key_file,
        iroh_key_file: paths
            .as_ref()
            .map(|p| p.iroh_path.display().to_string())
            .unwrap_or_default(),
        enc_key_file: paths
            .as_ref()
            .map(|p| p.enc_path.display().to_string())
            .unwrap_or_default(),
        sig_key_file: paths
            .as_ref()
            .map(|p| p.sig_path.display().to_string())
            .unwrap_or_default(),
        config_path: state.config_path.display().to_string(),
        data_root: state.data_root.display().to_string(),
    })
}

async fn get_config(State(state): State<AppState>) -> Json<ConfigResponse> {
    let cfg = state.cfg.read().await.clone();
    Json(ConfigResponse {
        ok: true,
        message: "runtime config".to_string(),
        config: cfg,
        config_path: state.config_path.display().to_string(),
        restart_required: false,
    })
}

async fn update_config(
    State(state): State<AppState>,
    Json(req): Json<UpdateConfigRequest>,
) -> Json<ConfigResponse> {
    let cfg_now = state.cfg.read().await.clone();
    let mut merged = cfg_now.clone();

    if let Some(alias) = req.kubo_key_alias.as_ref() {
        let alias = alias.trim();
        if !alias.is_empty() {
            merged.kubo_key_alias = alias.to_string();
        }
    }

    if let Some(lock_ttl) = req.lock_ttl {
        merged.lock_ttl = lock_ttl;
    }

    merged.normalize();

    let config_path = match persist_runtime_config(&state, merged.clone()).await {
        Ok(path) => path,
        Err(err) => {
            return Json(ConfigResponse {
                ok: false,
                message: format!("failed writing config: {}", err),
                config: cfg_now,
                config_path: state.config_path.display().to_string(),
                restart_required: false,
            });
        }
    };

    Json(ConfigResponse {
        ok: true,
        message: "config written and applied live".to_string(),
        config: merged,
        config_path: config_path.display().to_string(),
        restart_required: false,
    })
}

async fn validate_key_alias(
    State(state): State<AppState>,
    Query(query): Query<ValidateAliasQuery>,
) -> Json<ValidateAliasResponse> {
    let cfg = state.cfg.read().await.clone();
    let requested = query
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| kubo_key_alias(&cfg));

    match kubo_list_keys(&cfg.kubo_api_url).await {
        Ok(keys) => {
            let found = keys.iter().any(|k| k.name == requested);
            if found {
                Json(ValidateAliasResponse {
                    ok: true,
                    alias: requested,
                    found: true,
                    message: "kubo key alias exists".to_string(),
                })
            } else {
                Json(ValidateAliasResponse {
                    ok: false,
                    alias: requested.clone(),
                    found: false,
                    message: format!(
                        "kubo key alias '{}' was not found (manual setup required)",
                        requested
                    ),
                })
            }
        }
        Err(err) => Json(ValidateAliasResponse {
            ok: false,
            alias: requested,
            found: false,
            message: format!("failed listing kubo keys: {}", err),
        }),
    }
}

async fn list_agents(State(state): State<AppState>) -> Json<ListAgentsResponse> {
    let mut agents = Vec::new();

    if let Ok(entries) = fs::read_dir(&state.agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(meta) = serde_json::from_str::<AgentMeta>(&raw) else {
                continue;
            };
            agents.push(AgentInfoResponse {
                id: meta.id.clone(),
                did: meta.did.clone(),
                created_at: meta.created_at,
                updated_at: meta.updated_at,
                log_path: agent_log_path(&state, &meta.id).display().to_string(),
            });
        }
    }

    agents.sort_by(|a, b| a.id.cmp(&b.id));

    Json(ListAgentsResponse { ok: true, agents })
}

async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> Json<CreateAgentResponse> {
    let id = if let Some(explicit) = req.id {
        explicit.trim().to_string()
    } else {
        let mut chosen = generate_agent_id();
        let mut tries = 0usize;
        while agent_meta_path(&state, &chosen).exists() && tries < 8 {
            chosen = generate_agent_id();
            tries += 1;
        }
        chosen
    };

    if !is_valid_agent_id(&id) {
        return Json(CreateAgentResponse {
            ok: false,
            message: format!("invalid agent id '{}': expected [A-Za-z0-9_-]+", id),
            agent: None,
        });
    }

    let meta_path = agent_meta_path(&state, &id);
    if meta_path.exists() {
        return Json(CreateAgentResponse {
            ok: false,
            message: format!("agent '{}' already exists", id),
            agent: None,
        });
    }

    let world_did_root = match ensure_world_root_did_published(&state).await {
        Ok(value) => value,
        Err(err) => {
            return Json(CreateAgentResponse {
                ok: false,
                message: format!("failed ensuring world root DID publish: {}", err),
                agent: None,
            });
        }
    };

    let did = format!("{}#{}", world_did_root, id);

    let now = Utc::now().to_rfc3339();
    let meta = AgentMeta {
        id: id.clone(),
        did: did.clone(),
        signing_private_key_hex: None,
        encryption_private_key_hex: None,
        created_at: now.clone(),
        updated_at: now,
    };

    if let Err(err) = save_agent_meta(&state, &meta) {
        return Json(CreateAgentResponse {
            ok: false,
            message: format!("failed creating agent '{}': {}", id, err),
            agent: None,
        });
    }

    let log_path = agent_log_path(&state, &id);
    if let Err(err) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        return Json(CreateAgentResponse {
            ok: false,
            message: format!("failed creating log file '{}': {}", log_path.display(), err),
            agent: None,
        });
    }

    if let Err(err) = append_agent_log_line(&state, &id, "agent created") {
        return Json(CreateAgentResponse {
            ok: false,
            message: format!("failed writing initial log for '{}': {}", id, err),
            agent: None,
        });
    }

    Json(CreateAgentResponse {
        ok: true,
        message: format!("agent '{}' created", id),
        agent: Some(AgentInfoResponse {
            id,
            did,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
            log_path: log_path.display().to_string(),
        }),
    })
}

async fn delete_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<GenericResponse> {
    let id = id.trim().to_string();
    if !is_valid_agent_id(&id) {
        return Json(GenericResponse {
            ok: false,
            message: format!("invalid agent id '{}'", id),
        });
    }

    let meta_path = agent_meta_path(&state, &id);
    let log_path = agent_log_path(&state, &id);

    if meta_path.exists() {
        let _ = fs::remove_file(&meta_path);
    }
    if log_path.exists() {
        let _ = fs::remove_file(&log_path);
    }

    Json(GenericResponse {
        ok: true,
        message: format!("agent '{}' deleted", id),
    })
}

async fn get_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Json<LogsResponse> {
    let id = id.trim().to_string();
    let log_path = agent_log_path(&state, &id);

    let raw = fs::read_to_string(&log_path).unwrap_or_default();
    let mut lines = raw.lines().map(|line| line.to_string()).collect::<Vec<_>>();
    let total_scanned = lines.len();

    if let Some(q) = query.q.as_ref() {
        let needle = q.trim();
        if !needle.is_empty() {
            lines.retain(|line| line.contains(needle));
        }
    }

    let limit = query.limit.unwrap_or(200).clamp(1, 5000);
    if lines.len() > limit {
        lines = lines.split_off(lines.len() - limit);
    }

    Json(LogsResponse {
        ok: true,
        agent_id: id,
        lines,
        total_lines_scanned: total_scanned,
    })
}

async fn append_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AppendLogRequest>,
) -> Json<GenericResponse> {
    let id = id.trim().to_string();
    if !is_valid_agent_id(&id) {
        return Json(GenericResponse {
            ok: false,
            message: format!("invalid agent id '{}'", id),
        });
    }

    let mut meta = match load_agent_meta(&state, &id) {
        Ok(meta) => meta,
        Err(_) => {
            return Json(GenericResponse {
                ok: false,
                message: format!("agent '{}' not found", id),
            });
        }
    };

    let line = req.line.trim();
    if line.is_empty() {
        return Json(GenericResponse {
            ok: false,
            message: "line must not be empty".to_string(),
        });
    }

    if let Err(err) = append_agent_log_line(&state, &id, line) {
        return Json(GenericResponse {
            ok: false,
            message: err.to_string(),
        });
    }

    meta.updated_at = Utc::now().to_rfc3339();
    if let Err(err) = save_agent_meta(&state, &meta) {
        return Json(GenericResponse {
            ok: false,
            message: format!("failed updating metadata for '{}': {}", id, err),
        });
    }

    Json(GenericResponse {
        ok: true,
        message: format!("log appended for '{}'", id),
    })
}

fn world_root_from_did_text(input: &str) -> Option<String> {
    let did = Did::try_from(input.trim()).ok()?;
    Some(did.without_fragment().id())
}

fn build_agent_signing_key(root_did: &str, sig_private: &[u8]) -> Result<SigningKey> {
    let key_id = root_did
        .trim()
        .strip_prefix("did:ma:")
        .ok_or_else(|| anyhow!("invalid world DID root '{}'", root_did))?;
    let signing_did = Did::new(key_id, "sig")?;
    let key_bytes: [u8; 32] = sig_private
        .try_into()
        .map_err(|_| anyhow!("invalid signing key length"))?;
    SigningKey::from_private_key_bytes(signing_did, key_bytes).map_err(|e| anyhow!(e.to_string()))
}

async fn resolve_world_endpoint_id_from_did(kubo_url: &str, world_root_did: &str) -> Result<String> {
    let root = world_root_from_did_text(world_root_did)
        .ok_or_else(|| anyhow!("invalid world DID '{}'", world_root_did))?;
    let ipns_id = root
        .strip_prefix("did:ma:")
        .ok_or_else(|| anyhow!("invalid world DID root '{}'", root))?;
    let resolved = kubo_name_resolve(kubo_url, &format!("/ipns/{}", ipns_id)).await?;
    let doc = resolved_root_did_document(kubo_url, &resolved, &root)
        .await?
        .ok_or_else(|| anyhow!("resolved DID document mismatch for {}", root))?;

    let endpoint = doc
        .ma
        .as_ref()
        .and_then(|ma| {
            resolve_inbox_endpoint_id(
                ma.current_inbox.as_deref(),
                ma.presence_hint.as_deref(),
                ma.transports.as_ref(),
            )
        })
        .ok_or_else(|| anyhow!("world DID '{}' has no inbox endpoint", root))?;
    Ok(endpoint)
}

async fn send_world_request_over_iroh(endpoint_id: &str, request: WorldRequest) -> Result<WorldResponse> {
    let target: EndpointId = endpoint_id
        .trim()
        .parse()
        .map_err(|e| anyhow!("invalid endpoint id: {}", e))?;

    let endpoint = Endpoint::builder(presets::N0)
        .bind()
        .await
        .map_err(|e| anyhow!("endpoint bind failed: {}", e))?;
    let _ = endpoint.online().await;

    let relay_source = normalize_relay_url(DEFAULT_WORLD_RELAY_URL);
    let relay_url: RelayUrl = relay_source
        .parse()
        .map_err(|e| anyhow!("relay URL parse failed for '{}': {}", relay_source, e))?;
    let endpoint_addr = EndpointAddr::new(target).with_relay_url(relay_url);

    let connection = endpoint
        .connect(endpoint_addr, INBOX_ALPN)
        .await
        .map_err(|e| anyhow!("endpoint.connect() failed: {}", e))?;

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| anyhow!("connection.open_bi() failed: {}", e))?;

    let payload = serde_json::to_vec(&request)?;
    send.write_u32(payload.len() as u32).await?;
    send.write_all(&payload).await?;
    send.flush().await?;

    let frame_len = recv.read_u32().await? as usize;
    if frame_len > 1024 * 1024 {
        return Err(anyhow!("world response frame too large: {}", frame_len));
    }
    let mut bytes = vec![0u8; frame_len];
    recv.read_exact(&mut bytes).await?;

    let _ = send.finish();
    connection.close(0u32.into(), b"ok");
    endpoint.close().await;

    let response = serde_json::from_slice::<WorldResponse>(&bytes)
        .map_err(|e| anyhow!("invalid world response: {}", e))?;
    Ok(response)
}

async fn send_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SendAgentRequest>,
) -> Json<SendAgentResponse> {
    let id = id.trim().to_string();
    if !is_valid_agent_id(&id) {
        return Json(SendAgentResponse {
            ok: false,
            message: format!("invalid agent id '{}'", id),
            world_response: None,
        });
    }

    let mut meta = match load_agent_meta(&state, &id) {
        Ok(meta) => meta,
        Err(_) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("agent '{}' not found", id),
                world_response: None,
            });
        }
    };

    let body = req.body.trim();
    if body.is_empty() {
        return Json(SendAgentResponse {
            ok: false,
            message: "body must not be empty".to_string(),
            world_response: None,
        });
    }

    let mode = req.mode.as_deref().unwrap_or("command").trim().to_ascii_lowercase();
    if mode != "command" && mode != "chat" {
        return Json(SendAgentResponse {
            ok: false,
            message: format!("unsupported mode '{}': expected command|chat", mode),
            world_response: None,
        });
    }

    let cfg = state.cfg.read().await.clone();
    let sender_world_root = if !cfg.world_did_root.trim().is_empty() {
        cfg.world_did_root.trim().to_string()
    } else {
        match ensure_world_root_did_published(&state).await {
            Ok(root) => root,
            Err(err) => {
                return Json(SendAgentResponse {
                    ok: false,
                    message: format!("failed ensuring world DID root: {}", err),
                    world_response: None,
                });
            }
        }
    };

    let sender_did = format!("{}#{}", sender_world_root, id);
    if meta.did != sender_did {
        meta.did = sender_did.clone();
        meta.updated_at = Utc::now().to_rfc3339();
        let _ = save_agent_meta(&state, &meta);
    }

    let target_world_root = req
        .to
        .as_deref()
        .and_then(world_root_from_did_text)
        .unwrap_or_else(|| sender_world_root.clone());

    let endpoint_id = match resolve_world_endpoint_id_from_did(&cfg.kubo_api_url, &target_world_root).await {
        Ok(value) => value,
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed resolving world endpoint: {}", err),
                world_response: None,
            });
        }
    };

    let room = req.room.unwrap_or_else(|| "lobby".to_string()).trim().to_string();
    let envelope = if mode == "chat" {
        MessageEnvelope::Chatter {
            text: body.to_string(),
        }
    } else {
        let command_text = if body.starts_with('@') {
            body.to_string()
        } else if let Some(to) = req.to.as_deref() {
            format!("@{} {}", to.trim(), body)
        } else {
            body.to_string()
        };
        parse_message(&command_text)
    };

    let content = match serde_json::to_vec(&WorldCommand::Message {
        room,
        envelope,
    }) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed encoding world command: {}", err),
                world_response: None,
            });
        }
    };

    let secret_paths = match ensure_daemon_secret_files_for(&cfg, &state.config_path) {
        Ok(paths) => paths,
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed loading daemon secrets: {}", err),
                world_response: None,
            });
        }
    };
    let signing_secret = match ensure_secret_file(&secret_paths.sig_path, 32, "agent signing secret") {
        Ok(bytes) => bytes,
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed reading signing secret: {}", err),
                world_response: None,
            });
        }
    };
    let signing_key = match build_agent_signing_key(&sender_world_root, &signing_secret) {
        Ok(key) => key,
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed preparing signing key: {}", err),
                world_response: None,
            });
        }
    };

    let ttl = req.ttl_seconds.unwrap_or(60);
    let message = match did_ma::Message::new_with_ttl(
        sender_did.clone(),
        target_world_root.clone(),
        CONTENT_TYPE_WORLD,
        content,
        ttl,
        &signing_key,
    ) {
        Ok(msg) => msg,
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed building signed message: {}", err),
                world_response: None,
            });
        }
    };

    let request = match message.to_cbor() {
        Ok(message_cbor) => WorldRequest { message_cbor },
        Err(err) => {
            return Json(SendAgentResponse {
                ok: false,
                message: format!("failed encoding message cbor: {}", err),
                world_response: None,
            });
        }
    };

    match send_world_request_over_iroh(&endpoint_id, request).await {
        Ok(world_response) => {
            let _ = append_agent_log_line(
                &state,
                &id,
                &format!(
                    "sent mode={} to={} endpoint={} ok={}",
                    mode,
                    target_world_root,
                    endpoint_id,
                    world_response.ok
                ),
            );
            Json(SendAgentResponse {
                ok: true,
                message: "send ok".to_string(),
                world_response: Some(world_response),
            })
        }
        Err(err) => Json(SendAgentResponse {
            ok: false,
            message: format!("send failed: {}", err),
            world_response: None,
        }),
    }
}
