use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use ma_core::ObjectProgramRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluatorRef {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalKuboConfig {
    pub api_url: String,
    pub gateway_url: Option<String>,
    pub world_ipns_key_name: String,
    #[serde(default)]
    pub required_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalPathsConfig {
    pub authoring_dir: String,
    pub secrets_dir: String,
    pub state_dir: String,
    #[serde(default = "default_local_cache_dir")]
    pub cache_dir: String,
}

fn default_local_cache_dir() -> String {
    "cache".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalCryptoConfig {
    pub world_master_key_file: String,
    pub object_keys_file: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalPublishConfig {
    pub publish_world_ipns: bool,
    pub auto_pin: bool,
    pub room_ipns_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalRuntimeConfig {
    pub supported_evaluators: Vec<EvaluatorRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalWorldConfig {
    pub kind: String,
    pub version: u32,
    pub world_slug: String,
    pub world_id: String,
    pub kubo: LocalKuboConfig,
    pub paths: LocalPathsConfig,
    pub crypto: LocalCryptoConfig,
    pub publish: LocalPublishConfig,
    pub runtime: LocalRuntimeConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldTransportService {
    pub messaging: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldServices {
    pub builtin_verbs: Vec<String>,
    pub transport: WorldTransportService,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldManifestRefs {
    pub actor_registry_cid: String,
    pub room_index_cid: Option<String>,
    pub acl_index_cid: Option<String>,
    pub capabilities_index_cid: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldManifestMeta {
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorWebArtifact {
    pub version: String,
    pub cid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorWebRegistry {
    pub active_version: String,
    pub artifacts: HashMap<String, ActorWebArtifact>,
}

impl ActorWebRegistry {
    pub fn active_artifact(&self) -> Option<&ActorWebArtifact> {
        self.artifacts.get(&self.active_version)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldManifest {
    pub kind: String,
    pub version: u32,
    pub world_id: String,
    pub world_root_cid: String,
    pub entry_room_id: String,
    pub supported_evaluators: Vec<EvaluatorRef>,
    pub services: WorldServices,
    #[serde(default)]
    pub actor_web: Option<ActorWebRegistry>,
    pub refs: WorldManifestRefs,
    pub meta: WorldManifestMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldRootRefs {
    pub actor_registry_cid: String,
    pub room_index_cid: Option<String>,
    pub global_acl_cid: Option<String>,
    pub global_properties_cid: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldEntrypoints {
    pub default_room_id: String,
    pub spawn_location_id: Option<String>,
    pub system_actor_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldRootMeta {
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldRoot {
    pub kind: String,
    pub version: u32,
    pub world_id: String,
    pub refs: WorldRootRefs,
    pub entrypoints: WorldEntrypoints,
    pub meta: WorldRootMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorRegistryItem {
    pub cid: String,
    #[serde(rename = "type")]
    pub actor_type: String,
    pub owner: Option<String>,
    pub location: Option<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorRegistryIndexes {
    pub by_type: HashMap<String, Vec<String>>,
    pub by_location: HashMap<String, Vec<String>>,
    pub by_owner: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorRegistry {
    pub kind: String,
    pub version: u32,
    pub world_id: String,
    pub actors: HashMap<String, ActorRegistryItem>,
    pub indexes: ActorRegistryIndexes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorBuiltin {
    pub owner: Option<String>,
    pub location: Option<String>,
    pub controller: Option<String>,
    pub acl: Option<String>,
    pub program: ObjectProgramRef,
    pub payload_cid: Option<String>,
    pub payload_encrypted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorMeta {
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorIndexes {
    pub contents: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub kind: String,
    pub version: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub actor_type: String,
    pub builtin: ActorBuiltin,
    pub meta: ActorMeta,
    pub properties: serde_yaml::Value,
    pub indexes: ActorIndexes,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidationReport {
    pub world_dir: String,
    pub config_path: String,
    pub authoring_dir: String,
    pub actor_count: usize,
}

#[derive(Clone, Debug)]
pub struct LoadedWorldAuthoring {
    pub world_dir: PathBuf,
    pub config_path: PathBuf,
    pub authoring_dir: PathBuf,
    pub config: LocalWorldConfig,
    pub world_manifest: WorldManifest,
    pub world_root: WorldRoot,
    pub actor_registry: ActorRegistry,
    pub actors: HashMap<String, Actor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorSecretMaterial {
    pub signing_key_b64: String,
    pub encryption_key_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorSecretBundle {
    pub actor: Actor,
    pub secrets: ActorSecretMaterial,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SealedActorEnvelope {
    kind: String,
    version: u32,
    algorithm: String,
    nonce_b64: String,
    ciphertext_b64: String,
}

pub fn default_world_dir(world_slug: &str) -> PathBuf {
    worlds_data_root().join(world_slug)
}

pub fn worlds_data_root() -> PathBuf {
    if let Some(value) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(value).join("ma");
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("ma");
    }

    PathBuf::from(".config").join("ma")
}

pub fn validate_world_authoring(world_dir: &Path) -> Result<ValidationReport> {
    let loaded = load_world_authoring(world_dir)?;

    Ok(ValidationReport {
        world_dir: loaded.world_dir.display().to_string(),
        config_path: loaded.config_path.display().to_string(),
        authoring_dir: loaded.authoring_dir.display().to_string(),
        actor_count: loaded.actors.len(),
    })
}

pub fn load_world_authoring(world_dir: &Path) -> Result<LoadedWorldAuthoring> {
    let config_path = world_dir.join("config.yaml");
    let config = read_yaml::<LocalWorldConfig>(&config_path)?;
    ensure_schema(&config.kind, "local_world_config", config.version, "config.yaml")?;

    let authoring_dir = resolve_subpath(world_dir, &config.paths.authoring_dir);
    let world_manifest = read_yaml::<WorldManifest>(&authoring_dir.join("world_manifest.yaml"))?;
    ensure_schema(
        &world_manifest.kind,
        "world_manifest",
        world_manifest.version,
        "world_manifest.yaml",
    )?;

    let world_root = read_yaml::<WorldRoot>(&authoring_dir.join("world_root.yaml"))?;
    ensure_schema(&world_root.kind, "world_root", world_root.version, "world_root.yaml")?;

    let actor_registry = read_yaml::<ActorRegistry>(&authoring_dir.join("actor_registry.yaml"))?;
    ensure_schema(
        &actor_registry.kind,
        "actor_registry",
        actor_registry.version,
        "actor_registry.yaml",
    )?;

    let actors_dir = authoring_dir.join("actors");
    if !actors_dir.exists() {
        return Err(anyhow!("missing actors directory: {}", actors_dir.display()));
    }

    let mut actors = HashMap::new();
    for entry in fs::read_dir(&actors_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let actor = read_yaml::<Actor>(&path)?;
        ensure_schema(
            &actor.kind,
            "actor",
            actor.version,
            path.file_name().and_then(|s| s.to_str()).unwrap_or("actor"),
        )?;
        actors.insert(actor.id.clone(), actor);
    }

    if actors.is_empty() {
        return Err(anyhow!("no actor YAML files found in {}", actors_dir.display()));
    }

    let world_fragment = did_fragment(&world_manifest.world_id)
        .ok_or_else(|| anyhow!("world_id must include #fragment: {}", world_manifest.world_id))?;
    if world_fragment != config.kubo.world_ipns_key_name {
        return Err(anyhow!(
            "world_id fragment '{}' must equal kubo.world_ipns_key_name '{}'",
            world_fragment,
            config.kubo.world_ipns_key_name
        ));
    }

    if let Some(actor_web) = world_manifest.actor_web.as_ref() {
        validate_actor_web_registry(actor_web)?;
    }

    for actor in actors.values() {
        if actor.actor_type != "room" {
            continue;
        }
        let fragment = did_fragment(&actor.id)
            .ok_or_else(|| anyhow!("room actor id must include #fragment: {}", actor.id))?;
        if !fragment.starts_with(&format!("{}-", config.kubo.world_ipns_key_name)) {
            return Err(anyhow!(
                "room actor id fragment '{}' must start with '{}-'",
                fragment,
                config.kubo.world_ipns_key_name
            ));
        }
    }

    Ok(LoadedWorldAuthoring {
        world_dir: world_dir.to_path_buf(),
        config_path,
        authoring_dir,
        config,
        world_manifest,
        world_root,
        actor_registry,
        actors,
    })
}

pub fn did_fragment(id: &str) -> Option<&str> {
    id.split('#').nth(1).map(str::trim).filter(|s| !s.is_empty())
}

pub fn unlock_actor_secret_bundles(
    loaded: &LoadedWorldAuthoring,
) -> Result<HashMap<String, ActorSecretBundle>> {
    let key = read_world_master_key(&loaded.world_dir, &loaded.config)?;
    let secrets_dir = resolve_subpath(&loaded.world_dir, &loaded.config.paths.secrets_dir).join("actors");

    let mut bundles = HashMap::new();
    for actor in loaded.actors.values() {
        let fragment = did_fragment(&actor.id)
            .ok_or_else(|| anyhow!("actor id must include #fragment: {}", actor.id))?;
        let path = secrets_dir.join(format!("{}.sealed.yaml", fragment));
        if !path.exists() {
            return Err(anyhow!(
                "missing sealed actor secrets for {} at {}",
                actor.id,
                path.display()
            ));
        }

        let envelope = read_yaml::<SealedActorEnvelope>(&path)?;
        if envelope.kind != "sealed_actor_bundle" || envelope.version != 1 {
            return Err(anyhow!(
                "invalid sealed actor envelope in {}",
                path.display()
            ));
        }
        if envelope.algorithm != "xchacha20poly1305" {
            return Err(anyhow!(
                "unsupported sealed actor algorithm '{}' in {}",
                envelope.algorithm,
                path.display()
            ));
        }

        let nonce = B64
            .decode(&envelope.nonce_b64)
            .map_err(|e| anyhow!("invalid nonce in {}: {}", path.display(), e))?;
        if nonce.len() != 24 {
            return Err(anyhow!("invalid nonce size in {}", path.display()));
        }
        let ciphertext = B64
            .decode(&envelope.ciphertext_b64)
            .map_err(|e| anyhow!("invalid ciphertext in {}: {}", path.display(), e))?;

        let cipher = XChaCha20Poly1305::new((&key).into());
        let plaintext = cipher
            .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| anyhow!("failed to decrypt actor secrets in {}", path.display()))?;

        let bundle = serde_yaml::from_slice::<ActorSecretBundle>(&plaintext)
            .map_err(|e| anyhow!("invalid decrypted actor bundle in {}: {}", path.display(), e))?;
        if bundle.actor.id != actor.id {
            return Err(anyhow!(
                "sealed actor bundle mismatch in {}: expected '{}' got '{}'",
                path.display(),
                actor.id,
                bundle.actor.id
            ));
        }
        bundles.insert(actor.id.clone(), bundle);
    }

    Ok(bundles)
}

fn read_world_master_key(world_dir: &Path, config: &LocalWorldConfig) -> Result<[u8; 32]> {
    let path = resolve_subpath(world_dir, &config.crypto.world_master_key_file);
    let bytes = fs::read(&path)
        .map_err(|e| anyhow!("failed reading world master key {}: {}", path.display(), e))?;
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("world master key must be 32 bytes in {}", path.display()))?;
    Ok(key)
}

pub fn normalize_world_key_name(slug: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in slug.chars() {
        let keep = ch.to_ascii_lowercase();
        if keep.is_ascii_alphanumeric() {
            out.push(keep);
            prev_dash = false;
            continue;
        }
        if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }

    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "ma-world".to_string()
    } else {
        trimmed
    }
}

fn resolve_subpath(world_dir: &Path, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        world_dir.join(candidate)
    }
}

fn ensure_schema(kind: &str, expected_kind: &str, version: u32, context: &str) -> Result<()> {
    if kind != expected_kind {
        return Err(anyhow!(
            "invalid kind in {}: expected '{}' got '{}'",
            context,
            expected_kind,
            kind
        ));
    }
    if version != 1 {
        return Err(anyhow!(
            "invalid version in {}: expected 1 got {}",
            context,
            version
        ));
    }
    Ok(())
}

fn read_yaml<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = fs::read_to_string(path)
        .map_err(|e| anyhow!("failed reading {}: {}", path.display(), e))?;
    let value = serde_yaml::from_str::<T>(&raw)
        .map_err(|e| anyhow!("invalid YAML in {}: {}", path.display(), e))?;
    Ok(value)
}

fn validate_actor_web_registry(registry: &ActorWebRegistry) -> Result<()> {
    let active = registry.active_version.trim();
    if active.is_empty() {
        return Err(anyhow!("world_manifest actor_web.active_version must not be empty"));
    }
    if registry.artifacts.is_empty() {
        return Err(anyhow!("world_manifest actor_web.artifacts must not be empty"));
    }
    let Some(artifact) = registry.active_artifact() else {
        return Err(anyhow!(
            "world_manifest actor_web.active_version '{}' not found in actor_web.artifacts",
            active
        ));
    };
    if artifact.version.trim().is_empty() {
        return Err(anyhow!(
            "world_manifest actor_web artifact '{}' has empty version",
            active
        ));
    }
    if artifact.cid.trim().is_empty() {
        return Err(anyhow!(
            "world_manifest actor_web artifact '{}' has empty cid",
            active
        ));
    }
    Ok(())
}
