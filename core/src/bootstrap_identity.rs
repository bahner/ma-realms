#[cfg(not(target_arch = "wasm32"))]
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result, anyhow};
#[cfg(not(target_arch = "wasm32"))]
use libp2p_identity::Keypair;

#[cfg(not(target_arch = "wasm32"))]
use crate::{SecureFileKind, ensure_private_dir, write_secure_file};

#[cfg(not(target_arch = "wasm32"))]
pub fn default_ma_config_root() -> Result<PathBuf> {
    if let Ok(root) = env::var("XDG_CONFIG_HOME") {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            return Ok(Path::new(trimmed).join("ma"));
        }
    }

    let home = env::var("HOME").context("HOME env var is not set")?;
    Ok(Path::new(&home).join(".config").join("ma"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ensure_local_ipns_key_file(
    config_root: &Path,
    key_file_name: &str,
) -> Result<(Vec<u8>, PathBuf)> {
    if key_file_name.trim().is_empty() {
        return Err(anyhow!("key_file_name is required"));
    }

    ensure_private_dir(config_root)?;
    let keys_dir = config_root.join("keys");
    ensure_private_dir(&keys_dir)?;

    let key_path = keys_dir.join(key_file_name);
    if key_path.exists() {
        let existing = fs::read(&key_path).context("failed to read local ipns key")?;
        if !existing.is_empty() {
            return Ok((existing, key_path));
        }
    }

    let keypair = Keypair::generate_ed25519();
    let encoded = keypair
        .to_protobuf_encoding()
        .map_err(|e| anyhow!("failed to encode local ipns key: {}", e))?;

    write_secure_file(&key_path, &encoded, SecureFileKind::SensitiveData)?;

    Ok((encoded, key_path))
}