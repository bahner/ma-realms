use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

pub fn print_cli_help() {
    println!("ma-world");
    println!();
    println!("Usage:");
    println!("  ma-world run [--world-slug <slug>] [--listen <ip:port>] [--kubo-url <url>] [--cid <cid>] [--log-level <level>] [--log-file <path>]");
    println!("  ma-world publish-world --world-slug <slug> [--kubo-url <url>] [--skip-ipns] [--allow-partial-ipns] [--ipns-timeout-ms <ms>] [--ipns-retries <n>] [--ipns-backoff-ms <ms>] [--actor-web-cid <cid>]");
    println!("  ma-world set-kubo-url [--world-slug <slug>] --kubo-url <url>");
    println!("  ma-world ensure-world [--world-slug <slug>] [--kubo-url <url>] [--skip-ipns] [--allow-partial-ipns] [--ipns-timeout-ms <ms>] [--ipns-retries <n>] [--ipns-backoff-ms <ms>] [--actor-web-cid <cid>]");
    println!("  ma-world --gen-iroh-secret [--world-slug <slug>] [<path>]");
    println!();
    println!("publish-world options:");
    println!("  --world-slug <slug>");
    println!("  --kubo-url <url>");
    println!("  --actor-web-cid <cid>");
    println!("  --skip-ipns");
    println!("  --allow-partial-ipns");
    println!("  --ipns-timeout-ms <ms>");
    println!("  --ipns-retries <n>");
    println!("  --ipns-backoff-ms <ms>");
    println!();
    println!("run options (server mode):");
    println!("  --world-slug <slug>    Required world slug (e.g. panteia)");
    println!("  --listen <ip:port>");
    println!("  --kubo-url <url>");
    println!("  --cid <cid>            Override actor web CID for this run");
    println!("  --log-level <level>    Log level: trace, debug, info (default), warn, error");
    println!("  --log-file <path>      Write logs to file (appends to existing file)");
    println!("  runtime config file:   $XDG_CONFIG_HOME/ma/<slug>.yaml (or ~/.config/ma/<slug>.yaml)");
    println!("  iroh secret default:   $XDG_DATA_HOME/ma/iroh_<slug>_secret.bin");
    println!();
    println!("Environment variables:");
    println!("  MA_KUBO_API_URL               Kubo API URL");
    println!("  MA_LISTEN                     HTTP status listen socket");
    println!("  MA_LOG_LEVEL                  Log level for 'run' command");
    println!("  MA_LOG_FILE                   Log file path for 'run' command");
    println!();
    println!("Precedence (highest to lowest): CLI args > runtime config file > env vars > defaults");
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct RuntimeFileConfig {
    #[serde(default)]
    pub kubo_api_url: Option<String>,
    #[serde(default)]
    pub listen: Option<String>,
    #[serde(default)]
    pub iroh_secret: Option<String>,
    #[serde(default)]
    pub log_level: Option<String>,
    #[serde(default)]
    pub log_file: Option<String>,
    #[serde(default)]
    pub actor_web_version: Option<String>,
    #[serde(default)]
    pub actor_web_cid: Option<String>,
    #[serde(default)]
    pub actor_web_dir: Option<String>,
    #[serde(default)]
    pub actor_web_listen: Option<String>,
    #[serde(default)]
    pub actor_web_cache_dir: Option<String>,
    #[serde(default)]
    pub actor_web_ipns_key: Option<String>,
    #[serde(default)]
    pub actor_web_auto_build: Option<bool>,
    #[serde(default)]
    pub actor_web_auto_publish_ipns: Option<bool>,
}

fn xdg_config_home() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config");
    }
    PathBuf::from(".config")
}

pub fn xdg_data_home() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local").join("share");
    }
    PathBuf::from(".local").join("share")
}

pub fn runtime_config_path(world_slug: &str) -> PathBuf {
    xdg_config_home()
        .join("ma")
        .join(format!("{}.yaml", world_slug))
}

pub fn runtime_iroh_secret_default_path(world_slug: &str) -> PathBuf {
    xdg_data_home()
        .join("ma")
        .join(format!("iroh_{}_secret.bin", world_slug))
}

fn default_workspace_actor_web_dir() -> Option<PathBuf> {
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("ma-actor").join("www"));
    candidate.filter(|dir| dir.exists() && dir.is_dir())
}

pub fn resolve_actor_web_source_dir(runtime_cfg: &RuntimeFileConfig) -> Option<PathBuf> {
    if let Some(dir) = runtime_cfg
        .actor_web_dir
        .clone()
        .map(PathBuf::from)
        .filter(|dir| dir.exists() && dir.is_dir())
    {
        return Some(dir);
    }
    default_workspace_actor_web_dir()
}

pub fn load_runtime_file_config(path: &Path) -> Result<RuntimeFileConfig> {
    if !path.exists() {
        return Ok(RuntimeFileConfig::default());
    }
    let raw = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed reading runtime config {}: {}", path.display(), e))?;
    serde_yaml::from_str::<RuntimeFileConfig>(&raw)
        .map_err(|e| anyhow!("invalid runtime config {}: {}", path.display(), e))
}
