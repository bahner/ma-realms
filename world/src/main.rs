use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bootstrap::{
    load_runtime_file_config, print_cli_help, resolve_actor_web_source_dir, runtime_config_path,
    runtime_iroh_secret_default_path, xdg_data_home,
};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use chrono::Utc;
use did_ma::{Did, Document, EncryptionKey, Message, SigningKey, VerificationMethod};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey,
    endpoint::Connection,
    endpoint::presets,
    protocol::{AcceptError, ProtocolHandler, Router},
};
use ma_core::{
    ActorCommand, BROADCAST_ALPN, CHAT_ALPN, CMD_ALPN, CONTENT_TYPE_CHAT, CONTENT_TYPE_CMD,
    CLOSET_ALPN, ClosetRequest, ClosetResponse,
    ClosetCommand, ClosetProfileField, ClosetRecoveryCommand,
    ClosetDidPublishPlan, ensure_issued_document_root_match, ensure_session_document_root_match,
    CONTENT_TYPE_BROADCAST, CONTENT_TYPE_PRESENCE, CONTENT_TYPE_WORLD, CompiledCapabilityAcl,
    DEFAULT_WORLD_RELAY_URL,
    ExitData, LaneCapability, MessageEnvelope, ObjectDefinition, ObjectInboxMessage,
    CLOSET_COMMANDS_INLINE, MAILBOX_COMMANDS_INLINE,
    ObjectMessageIntent, ObjectMessageKind, ObjectMessageRetention, ObjectMessageTarget,
    ObjectRuntimeState, PRESENCE_ALPN, PresenceAvatar, RoomActorAction, RoomActorContext,
    RoomEvent, TransportAck, TransportAckCode, WORLD_ALPN, WorldCommand, WorldLane, WorldRequest,
    WorldResponse, compile_acl, evaluate_compiled_acl_with_owner, execute_room_actor_command,
    normalize_spoken_text, parse_capability_acl_text, parse_object_local_capability_acl,
    parse_closet_command, plan_closet_did_publish,
    LegacyRequirement, RequirementChecker, RequirementSet, RequirementValue,
};
use nanoid::nanoid;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{net::TcpListener, sync::RwLock};
use tracing::{debug, error, info, warn};
use tracing_subscriber::prelude::*;

mod actor;
mod actor_web;
mod bootstrap;
mod content_validation;
mod kubo;
mod lang;
mod room;
mod schema;
mod status;

use actor::Avatar;
use actor_web::{
    materialize_actor_web_from_cid, publish_actor_web_from_dir, resolve_actor_web_cid_from_ipns_key,
};
use lang::{
    collapse_world_language_order_strict,
    supported_world_languages_text,
    tr_world,
    tr_world_vars,
    world_lang_from_profile,
};
use kubo::{
    IpnsPublishOptions, dag_get_dag_cbor, dag_put_dag_cbor, generate_kubo_key, ipfs_add,
    import_kubo_key, ipns_publish_with_retry, list_kubo_key_names, list_kubo_keys, name_resolve, pin_add_named,
    pin_rm, pin_update, wait_for_kubo_api,
    remove_kubo_key,
};
use room::{Room, RoomAcl};
use schema::{
    ActorSecretBundle, default_world_dir, did_fragment, load_world_authoring,
    normalize_world_key_name, unlock_actor_secret_bundles, validate_world_authoring,
};

const DEFAULT_ROOM: &str = "lobby";
const DEFAULT_ENTRY_ACL: &str = "*";
const WORLD_ENTRY_ACL_ENV: &str = "MA_WORLD_ENTRY_ACL";
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:5002";
const MAX_EVENTS: usize = 200;
const MAX_KNOCK_INBOX: usize = 512;
#[allow(dead_code)]
const MAX_OBJECT_INBOX: usize = 512;
const MAILBOX_LOCK_SECS: u64 = 600;
const OBJECT_WASHER_INTERVAL_SECS: u64 = 20;
const CLOSET_LISTENING_MESSAGE: &str = "someone appears to be listening from the closet";
const RELAY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_WORLD_SLUG: &str = "ma";
const DEFAULT_KUBO_API_URL: &str = "http://127.0.0.1:5001";

fn extract_global_config_arg(raw_args: Vec<String>) -> Result<Vec<String>> {
    if raw_args.iter().any(|arg| arg == "--config") {
        return Err(anyhow!(
            "--config has been removed; pass runtime options directly via CLI/env"
        ));
    }
    if raw_args.iter().any(|arg| arg == "--world-dir") {
        return Err(anyhow!("--world-dir has been removed"));
    }
    if raw_args.get(1).map(String::as_str) == Some("print-config") {
        return Err(anyhow!("print-config has been removed"));
    }
    if raw_args.get(1).map(String::as_str) == Some("print-effective-config") {
        return Err(anyhow!("print-effective-config has been removed"));
    }
    Ok(raw_args)
}

#[derive(Clone, Debug)]
struct CachedDidDocument {
    document: Document,
    dirty: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AvatarRequest {
    pub inbox: String,
    pub did: Did,
    pub agent_endpoint: String,
    pub language_order: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AvatarSnapshot {
    pub inbox: String,
    pub agent_did: String,
    pub agent_endpoint: String,
    pub owner: String,
    pub description: String,
    pub acl: String,
    pub joined_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RoomSnapshot {
    pub name: String,
    pub avatars: Vec<AvatarSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldSnapshot {
    pub rooms: Vec<RoomSnapshot>,
    pub recent_events: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldInfo {
    pub name: String,
    pub world_did: String,
    pub status_url: String,
    pub endpoint_id: String,
    pub direct_addresses: Vec<String>,
    pub multiaddrs: Vec<String>,
    pub relay_urls: Vec<String>,
    pub kubo_url: String,
    pub location_hint: String,
    pub entry_acl: String,
    pub started_at: String,
    pub capabilities: Vec<LaneCapability>,
    pub actor_web: Option<ActorWebInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActorWebInfo {
    pub version: Option<String>,
    pub cid: Option<String>,
    pub status_url: String,
    pub source_dir: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OutboxRequest {
    Signed { message_cbor: Vec<u8> },
}

#[derive(Debug, Deserialize, Serialize)]
struct OutboxResponse {
    ok: bool,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct PresenceSnapshotEvent {
    v: u8,
    kind: String,
    room: String,
    room_did: String,
    room_title: String,
    room_description: String,
    avatars: Vec<PresenceAvatar>,
    seq: u64,
    ts: String,
}

#[derive(Clone, Debug, Serialize)]
struct RoomBroadcastEvent {
    v: u8,
    kind: String,
    room: String,
    room_did: String,
    sender: Option<String>,
    message: String,
    seq: u64,
    ts: String,
}



#[derive(Clone, Debug)]
struct WorldProtocol {
    world: Arc<World>,
    endpoint: Endpoint,
    endpoint_id: String,
    did_cache: Arc<RwLock<HashMap<String, CachedDidDocument>>>,
    lane: WorldLane,
}

#[derive(Clone, Debug)]
struct ClosetProtocol {
    world: Arc<World>,
}

#[derive(Clone, Debug)]
struct ClosetSession {
    id: String,
    endpoint: String,
    created_at: String,
    last_lobby_sequence: u64,
    announced_listening: bool,
    name: Option<String>,
    description: Option<String>,
    alias: Option<String>,
    did: Option<String>,
    fragment: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct ClosetProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preferred_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_checksum: Option<String>,
    updated_at: String,
}

#[derive(Clone, Debug, Default)]
struct ObjectRequirementRuntime {
    room_name: String,
    user: String,
    owner: Option<String>,
    location: String,
    opened_by: Option<String>,
    world_owner: Option<String>,
}

impl RequirementChecker for ObjectRequirementRuntime {
    fn resolve_symbol(&self, symbol: &str) -> Option<RequirementValue> {
        match symbol {
            "user" => Some(RequirementValue::String(self.user.clone())),
            "owner" => Some(
                self.owner
                    .clone()
                    .map(RequirementValue::String)
                    .unwrap_or(RequirementValue::Null),
            ),
            "location" => Some(RequirementValue::String(self.location.clone())),
            "opened_by" => Some(
                self.opened_by
                    .clone()
                    .map(RequirementValue::String)
                    .unwrap_or(RequirementValue::Null),
            ),
            "world.owner" => Some(
                self.world_owner
                    .clone()
                    .map(RequirementValue::String)
                    .unwrap_or(RequirementValue::Null),
            ),
            "world.slug" => Some(RequirementValue::String(DEFAULT_WORLD_SLUG.to_string())),
            _ => {
                if let Some(state_key) = symbol.strip_prefix("state.") {
                    let _ = state_key;
                    return Some(RequirementValue::Null);
                }
                None
            }
        }
    }

    fn check_legacy_requirement(&self, requirement: &LegacyRequirement) -> bool {
        match requirement.name.as_str() {
            "object.opened_by_self" => self
                .opened_by
                .as_ref()
                .map(|did| did == &self.user)
                .unwrap_or(false),
            "world.owned" => self
                .world_owner
                .as_ref()
                .map(|owner| owner == &self.user)
                .unwrap_or(false),
            "room.in" => requirement
                .arg
                .as_ref()
                .map(|room| room.trim().eq_ignore_ascii_case(self.room_name.as_str()))
                .unwrap_or(true),
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
struct EntryAcl {
    allow_all: bool,
    allow_owner: bool,
    allowed_dids: HashSet<String>,
    source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum KnockStatus {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct KnockMessage {
    id: u64,
    room: String,
    requester_did: String,
    requester_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preferred_handle: Option<String>,
    requested_at: String,
    status: KnockStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decided_at: Option<String>,
}

#[derive(Debug)]
pub struct World {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    events: Arc<RwLock<VecDeque<String>>>,
    room_events: Arc<RwLock<HashMap<String, VecDeque<RoomEvent>>>>,
    next_room_event_sequence: Arc<RwLock<u64>>,
    entry_acl: Arc<RwLock<EntryAcl>>,
    /// handle (string) → root DID.  Prevents two different DIDs sharing a handle.
    handle_to_did: Arc<RwLock<HashMap<String, String>>>,
    /// root DID → assigned handle.  Lets the same DID re-enter with the same handle.
    did_to_handle: Arc<RwLock<HashMap<String, String>>>,
    /// Actor runtime key material loaded from sealed actor bundles.
    actor_secrets: Arc<RwLock<HashMap<String, RuntimeActorSecret>>>,
    /// World owner root DID. Managed from status API.
    owner_did: Arc<RwLock<Option<String>>>,
    /// Kubo API base URL, mutable at runtime from status UI.
    kubo_url: Arc<RwLock<String>>,
    /// room_name → IPFS CID of the last persisted room YAML.
    room_cids: Arc<RwLock<HashMap<String, String>>>,
    /// CID of the current world root index (if known).
    world_cid: Arc<RwLock<Option<String>>>,
    /// Root DID for this world, sourced from authored world config.
    world_did_root: Arc<RwLock<Option<String>>>,
    /// Full world DID (with fragment) from authored world config.
    world_did: Arc<RwLock<Option<String>>>,
    /// Runtime state lock; when false, command lanes reject world interactions.
    unlocked: Arc<RwLock<bool>>,
    /// Global capability ACL (typically loaded from world_root.refs.global_acl_cid).
    global_capability_acl: Arc<RwLock<Option<CompiledCapabilityAcl>>>,
    /// Source marker for currently loaded global capability ACL.
    global_capability_acl_source: Arc<RwLock<Option<String>>>,
    /// Compiled capability ACL cache keyed by ACL CID.
    capability_acl_cache: Arc<RwLock<HashMap<String, CompiledCapabilityAcl>>>,
    /// Authored world directory used to unlock sealed actor bundles.
    unlock_world_dir: Arc<RwLock<Option<PathBuf>>>,
    /// Path to world master key file for encrypted state save/load.
    world_master_key_path: Arc<RwLock<Option<PathBuf>>>,
    /// Master key decrypted from passphrase+bundle at runtime.
    unlocked_world_master_key: Arc<RwLock<Option<[u8; 32]>>>,
    /// World signing key material used for DID assertions.
    unlocked_world_signing_key: Arc<RwLock<Option<[u8; 32]>>>,
    /// World encryption key material used for persisted runtime-state encryption.
    unlocked_world_encryption_key: Arc<RwLock<Option<[u8; 32]>>>,
    /// CID of the latest encrypted runtime state envelope.
    state_cid: Arc<RwLock<Option<String>>>,
    /// CID of the active language package manifest for this world.
    lang_cid: Arc<RwLock<Option<String>>>,
    /// Stable Kubo pin name for world root index snapshots.
    world_root_pin_name: Arc<RwLock<String>>,
    /// Last result of publishing world root CID to the runtime state pointer IPNS key.
    last_pointer_publish_ok: Arc<RwLock<Option<bool>>>,
    last_pointer_publish_root_cid: Arc<RwLock<Option<String>>>,
    last_pointer_publish_error: Arc<RwLock<Option<String>>>,
    /// Room-local interactable objects keyed by room then object id.
    room_objects: Arc<RwLock<HashMap<String, HashMap<String, ObjectRuntimeState>>>>,
    /// Inbox of async knock requests for private worlds.
    knock_inbox: Arc<RwLock<VecDeque<KnockMessage>>>,
    /// Monotonic knock id sequence.
    next_knock_id: Arc<RwLock<u64>>,
    /// Ephemeral pre-DID onboarding sessions keyed by session id.
    closet_sessions: Arc<RwLock<HashMap<String, ClosetSession>>>,
    /// DID-root keyed profile data established via closet onboarding.
    closet_profiles: Arc<RwLock<HashMap<String, ClosetProfile>>>,
}

#[derive(Clone, Debug)]
struct RuntimeActorSecret {
    signing_key: [u8; 32],
}

use argon2::{Algorithm, Argon2, Params, Version};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorldAccessBundle {
    version: u32,
    kdf: String,
    salt_b64: String,
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorldAccessBundlePlain {
    version: u32,
    world_master_key_b64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    world_signing_private_key_b64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    world_encryption_private_key_b64: Option<String>,
}

#[derive(Clone, Debug)]
struct WorldRuntimeSecrets {
    world_master_key: [u8; 32],
    world_signing_private_key: [u8; 32],
    world_encryption_private_key: [u8; 32],
}

fn derive_bundle_key_argon2(password: &[u8], salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(19456, 2, 1, Some(32))
        .map_err(|e| anyhow!("argon2 params: {}", e))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(password, salt, &mut out)
        .map_err(|e| anyhow!("argon2 key derivation failed: {}", e))?;
    Ok(out)
}

fn decrypt_world_access_bundle(passphrase: &str, bundle_json: &str) -> Result<WorldRuntimeSecrets> {
    let bundle: WorldAccessBundle = serde_json::from_str(bundle_json)
        .map_err(|e| anyhow!("invalid bundle JSON: {}", e))?;

    if bundle.version != 1 || bundle.kdf != "argon2id" {
        return Err(anyhow!("unsupported bundle version or kdf"));
    }

    let salt = B64
        .decode(bundle.salt_b64.as_bytes())
        .map_err(|e| anyhow!("invalid bundle salt: {}", e))?;
    let nonce_raw = B64
        .decode(bundle.nonce_b64.as_bytes())
        .map_err(|e| anyhow!("invalid bundle nonce: {}", e))?;
    let nonce: [u8; 24] = nonce_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid bundle nonce length"))?;
    let ciphertext = B64
        .decode(bundle.ciphertext_b64.as_bytes())
        .map_err(|e| anyhow!("invalid bundle ciphertext: {}", e))?;

    let key = derive_bundle_key_argon2(passphrase.as_bytes(), &salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let plaintext = cipher
        .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| anyhow!("wrong passphrase or corrupted bundle"))?;

    let plain: WorldAccessBundlePlain = serde_json::from_slice(&plaintext)
        .map_err(|e| anyhow!("invalid decrypted bundle payload: {}", e))?;
    if plain.version != 1 && plain.version != 2 {
        return Err(anyhow!("unsupported bundle payload version"));
    }

    let key_raw = B64
        .decode(plain.world_master_key_b64.as_bytes())
        .map_err(|e| anyhow!("invalid world_master_key in bundle: {}", e))?;
    let world_master_key: [u8; 32] = key_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("world_master_key in bundle must be 32 bytes"))?;

    let world_signing_private_key = if let Some(value) = plain.world_signing_private_key_b64 {
        let raw = B64
            .decode(value.as_bytes())
            .map_err(|e| anyhow!("invalid world_signing_private_key in bundle: {}", e))?;
        raw.as_slice()
            .try_into()
            .map_err(|_| anyhow!("world_signing_private_key in bundle must be 32 bytes"))?
    } else {
        derive_world_signing_private_key(&world_master_key)
    };

    let world_encryption_private_key = if let Some(value) = plain.world_encryption_private_key_b64 {
        let raw = B64
            .decode(value.as_bytes())
            .map_err(|e| anyhow!("invalid world_encryption_private_key in bundle: {}", e))?;
        raw.as_slice()
            .try_into()
            .map_err(|_| anyhow!("world_encryption_private_key in bundle must be 32 bytes"))?
    } else {
        derive_world_encryption_private_key(&world_master_key)
    };

    Ok(WorldRuntimeSecrets {
        world_master_key,
        world_signing_private_key,
        world_encryption_private_key,
    })
}
fn is_valid_nanoid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn normalize_local_object_id(input: &str) -> String {
    input
        .trim()
        .trim_start_matches('#')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

/// Root index persisted in IPFS: maps room_name → room CID.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct WorldRootIndex {
    rooms: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IpldLink {
    #[serde(rename = "/")]
    cid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct WorldRootRoomEntry {
    #[serde(rename = "/")]
    cid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    did: Option<String>,
    owner_cid: String,
    acl_cid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum WorldRootRoomDagValue {
    Link(IpldLink),
    Entry(WorldRootRoomEntry),
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct WorldRootIndexDag {
    rooms: HashMap<String, WorldRootRoomDagValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state_cid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lang_cid: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedWorldEnvelope {
    kind: String,
    version: u32,
    created_at: String,
    signer_did: String,
    signature_b64: String,
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RoomAclDoc {
    kind: String,
    version: u32,
    acl: RoomAcl,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RoomOwnerDoc {
    kind: String,
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_assertion_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AvatarStateDoc {
    inbox: String,
    agent_did: String,
    agent_endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    language_order: String,
    owner: String,
    descriptions: HashMap<String, String>,
    acl: actor::ActorAcl,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RoomStateDoc {
    name: String,
    #[serde(default)]
    titles: HashMap<String, String>,
    exits: Vec<ExitData>,
    descriptions: HashMap<String, String>,
    did: String,
    #[serde(default)]
    avatars: Vec<AvatarStateDoc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RuntimeStateDoc {
    kind: String,
    version: u32,
    rooms: HashMap<String, RoomStateDoc>,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    room_events: HashMap<String, Vec<RoomEvent>>,
    next_room_event_sequence: u64,
    #[serde(default)]
    handle_to_did: HashMap<String, String>,
    #[serde(default)]
    did_to_handle: HashMap<String, String>,
    owner_did: Option<String>,
    #[serde(default)]
    room_cids: HashMap<String, String>,
    #[serde(default)]
    room_objects: HashMap<String, Vec<ObjectRuntimeState>>,
    #[serde(default)]
    closet_profiles: HashMap<String, ClosetProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lang_cid: Option<String>,
}

fn build_exit_entry(id: String, name: String, to: String) -> ExitData {
    let normalized = "und".to_string();
    let mut exit = ExitData::new(id, name.clone(), to);
    exit.names.clear();
    if !name.trim().is_empty() {
        exit.names.insert(normalized, name);
    }
    exit
}

fn sender_profile_from_document(document: &Document) -> String {
    if let Some(ma) = document.ma.as_ref() {
        if let Some(language) = ma.language.as_ref() {
            let normalized = language.trim();
            if !normalized.is_empty() {
                return normalized.to_string();
            }
        }
    }
    "und".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ExitYamlDoc {
    kind: String,
    version: u32,
    exit: ExitData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RoomYamlDocV2 {
    kind: String,
    version: u32,
    id: String,
    #[serde(default)]
    titles: HashMap<String, String>,
    #[serde(default)]
    descriptions: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    did: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    exits: Vec<ExitData>,
    #[serde(default)]
    exit_cids: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct LegacyRoomAclYaml {
    owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_assertion_key: Option<String>,
    #[serde(default)]
    allow_all: bool,
    #[serde(default)]
    allow: HashSet<String>,
    #[serde(default)]
    deny: HashSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct LegacyRoomYaml {
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    exits: Vec<ExitData>,
    #[serde(default)]
    acl: LegacyRoomAclYaml,
    #[serde(default)]
    descriptions: HashMap<String, String>,
    #[serde(default)]
    did: String,
}

impl World {
    async fn closet_lang_for_session(&self, session_id: &str, endpoint: &str) -> &'static str {
        let session = {
            let sessions = self.closet_sessions.read().await;
            sessions.get(session_id).cloned()
        };

        if let Some(session) = session {
            if let Some(session_did) = session.did.as_deref() {
                if let Ok(did) = Did::try_from(session_did) {
                    let did_root = did.without_fragment().id();
                    if let Some(profile) = self.avatar_language_order_for_did(DEFAULT_ROOM, did_root.as_str()).await {
                        return world_lang_from_profile(&profile);
                    }
                }
            }
        }

        if let Some(did_root) = self.did_root_by_endpoint(DEFAULT_ROOM, endpoint).await {
            if let Some(profile) = self
                .avatar_language_order_for_did(DEFAULT_ROOM, did_root.as_str())
                .await
            {
                return world_lang_from_profile(&profile);
            }
        }

        world_lang_from_profile("nb_NO:en_UK")
    }

    fn recovery_checksum(secret: &str) -> String {
        let normalized = secret.trim();
        let mut hasher = Sha256::new();
        hasher.update(b"ma/closet/recovery/v1");
        hasher.update(normalized.as_bytes());
        B64.encode(hasher.finalize())
    }

    pub(crate) fn new(entry_acl: EntryAcl, kubo_url: String, world_root_pin_name: String) -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_EVENTS))),
            room_events: Arc::new(RwLock::new(HashMap::new())),
            next_room_event_sequence: Arc::new(RwLock::new(0)),
            entry_acl: Arc::new(RwLock::new(entry_acl)),
            handle_to_did: Arc::new(RwLock::new(HashMap::new())),
            did_to_handle: Arc::new(RwLock::new(HashMap::new())),
            actor_secrets: Arc::new(RwLock::new(HashMap::new())),
            owner_did: Arc::new(RwLock::new(None)),
            kubo_url: Arc::new(RwLock::new(kubo_url)),
            room_cids: Arc::new(RwLock::new(HashMap::new())),
            world_cid: Arc::new(RwLock::new(None)),
            world_did_root: Arc::new(RwLock::new(None)),
            world_did: Arc::new(RwLock::new(None)),
            unlocked: Arc::new(RwLock::new(false)),
            global_capability_acl: Arc::new(RwLock::new(None)),
            global_capability_acl_source: Arc::new(RwLock::new(None)),
            capability_acl_cache: Arc::new(RwLock::new(HashMap::new())),
            unlock_world_dir: Arc::new(RwLock::new(None)),
            world_master_key_path: Arc::new(RwLock::new(None)),
            unlocked_world_master_key: Arc::new(RwLock::new(None)),
            unlocked_world_signing_key: Arc::new(RwLock::new(None)),
            unlocked_world_encryption_key: Arc::new(RwLock::new(None)),
            state_cid: Arc::new(RwLock::new(None)),
            lang_cid: Arc::new(RwLock::new(None)),
            world_root_pin_name: Arc::new(RwLock::new(world_root_pin_name)),
            last_pointer_publish_ok: Arc::new(RwLock::new(None)),
            last_pointer_publish_root_cid: Arc::new(RwLock::new(None)),
            last_pointer_publish_error: Arc::new(RwLock::new(None)),
            room_objects: Arc::new(RwLock::new(HashMap::new())),
            knock_inbox: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_KNOCK_INBOX))),
            next_knock_id: Arc::new(RwLock::new(0)),
            closet_sessions: Arc::new(RwLock::new(HashMap::new())),
            closet_profiles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn world_root_pin_name(&self) -> String {
        self.world_root_pin_name.read().await.clone()
    }

    async fn issue_bootstrap_did(
        &self,
        requester_endpoint: &str,
        desired_alias: Option<&str>,
    ) -> Result<(String, String, String)> {
        let kubo_url = self.kubo_url.read().await.clone();
        let existing_names = list_kubo_key_names(&kubo_url).await?;

        let fragment = if let Some(alias) = desired_alias {
            let normalized = alias.trim();
            if normalized.is_empty() {
                return Err(anyhow!("alias cannot be empty"));
            }
            if !is_valid_nanoid_id(normalized) {
                return Err(anyhow!(
                    "invalid alias '{}': expected [A-Za-z0-9_-]+",
                    normalized
                ));
            }
            if existing_names.iter().any(|name| name == normalized) {
                return Err(anyhow!(
                    "fragment '{}' exists already; choose another alias/fragment and try again",
                    normalized
                ));
            }
            normalized.to_string()
        } else {
            let mut generated: Option<String> = None;
            for _ in 0..64 {
                let candidate = nanoid!();
                if !existing_names.iter().any(|name| name == &candidate) {
                    generated = Some(candidate);
                    break;
                }
            }
            generated.ok_or_else(|| anyhow!("failed to allocate a unique fragment"))?
        };

        generate_kubo_key(&kubo_url, &fragment).await?;
        let keys = list_kubo_keys(&kubo_url).await?;
        let did_ipns = keys
            .iter()
            .find(|key| key.name == fragment)
            .map(|key| key.id.trim().to_string())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("kubo key '{}' created but missing usable Id", fragment))?;

        let did = Did::new_root(&did_ipns)
            .map_err(|e| anyhow!("failed to construct did from key id '{}': {}", did_ipns, e))?
            .id();

        let did_obj = Did::try_from(did.as_str())
            .map_err(|e| anyhow!("failed to parse generated did '{}': {}", did, e))?;
        if !self.can_enter(&did_obj).await {
            let _ = remove_kubo_key(&kubo_url, &fragment).await;
            let did_root = did_obj.without_fragment().id();
            let (knock_id, duplicate) = self
                .enqueue_knock(DEFAULT_ROOM, &did_root, requester_endpoint, None)
                .await;
            let message = if duplicate {
                format!(
                    "entry denied for {}; existing knock request id={} is pending",
                    did_root, knock_id
                )
            } else {
                format!(
                    "entry denied for {}; knock request queued with id={}",
                    did_root, knock_id
                )
            };
            return Err(anyhow!(message));
        }

        Ok((did, fragment.clone(), fragment))
    }

    async fn closet_start_session(&self, endpoint: &str) -> Result<(String, u64)> {
        let session_id = nanoid!();
        let latest_lobby_sequence = self.latest_room_event_sequence(DEFAULT_ROOM).await?;
        let mut profile_name: Option<String> = None;
        let mut profile_description: Option<String> = None;
        let mut profile_alias: Option<String> = None;
        let mut profile_did: Option<String> = None;

        if let Some(did_root) = self.did_root_by_endpoint(DEFAULT_ROOM, endpoint).await {
            profile_did = Some(did_root.clone());
            if let Some(profile) = self
                .closet_profiles
                .read()
                .await
                .get(&did_root)
                .cloned()
            {
                profile_name = profile.name;
                profile_description = profile.description;
                profile_alias = profile.preferred_alias;
            }
        }

        let session = ClosetSession {
            id: session_id.clone(),
            endpoint: endpoint.to_string(),
            created_at: Utc::now().to_rfc3339(),
            last_lobby_sequence: latest_lobby_sequence,
            announced_listening: false,
            name: profile_name,
            description: profile_description,
            alias: profile_alias,
            did: profile_did,
            fragment: None,
        };
        self.closet_sessions
            .write()
            .await
            .insert(session_id.clone(), session);
        self.announce_closet_listener(session_id.as_str(), endpoint).await?;
        Ok((session_id, latest_lobby_sequence))
    }

    async fn announce_closet_listener(&self, session_id: &str, endpoint: &str) -> Result<()> {
        let mut sessions = self.closet_sessions.write().await;
        let Some(session) = sessions.get_mut(session_id) else {
            return Err(anyhow!("unknown closet session"));
        };
        if session.endpoint != endpoint {
            return Err(anyhow!("closet session endpoint mismatch"));
        }
        if session.announced_listening {
            return Ok(());
        }
        session.announced_listening = true;
        drop(sessions);

        self.record_room_event(
            DEFAULT_ROOM,
            "system",
            None,
            None,
            Some(endpoint.to_string()),
            CLOSET_LISTENING_MESSAGE.to_string(),
        )
        .await;
        Ok(())
    }

    async fn closet_session_owned_by(
        &self,
        session_id: &str,
        endpoint: &str,
    ) -> Result<ClosetSession> {
        let sessions = self.closet_sessions.read().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| anyhow!("unknown closet session"))?;
        if session.endpoint != endpoint {
            return Err(anyhow!("closet session endpoint mismatch"));
        }
        Ok(session.clone())
    }

    async fn closet_answer(
        &self,
        session_id: &str,
        endpoint: &str,
        field: &str,
        value: &str,
    ) -> Result<String> {
        let mut sessions = self.closet_sessions.write().await;
        let Some(session) = sessions.get_mut(session_id) else {
            return Err(anyhow!("unknown closet session"));
        };
        if session.endpoint != endpoint {
            return Err(anyhow!("closet session endpoint mismatch"));
        }

        let normalized_field = field.trim().to_ascii_lowercase();
        let normalized_value = value.trim().to_string();
        match normalized_field.as_str() {
            "name" => {
                if normalized_value.is_empty() {
                    return Err(anyhow!("name cannot be empty"));
                }
                session.name = Some(normalized_value);
                Ok("avatar name stored".to_string())
            }
            "description" | "desc" => {
                if normalized_value.is_empty() {
                    return Err(anyhow!("description cannot be empty"));
                }
                session.description = Some(normalized_value);
                Ok("avatar description stored".to_string())
            }
            _ => Err(anyhow!("unknown closet field '{}': use name|description", field)),
        }
    }

    async fn upsert_closet_profile(
        &self,
        did_raw: &str,
        name: Option<String>,
        description: Option<String>,
        preferred_alias: Option<String>,
    ) -> Result<()> {
        let did = Did::try_from(did_raw)
            .map_err(|e| anyhow!("invalid closet profile DID '{}': {}", did_raw, e))?;
        let did_root = did.without_fragment().id();

        let mut profiles = self.closet_profiles.write().await;
        let profile = profiles.entry(did_root).or_default();
        if let Some(value) = name {
            let normalized = value.trim().to_string();
            if !normalized.is_empty() {
                profile.name = Some(normalized);
            }
        }
        if let Some(value) = description {
            let normalized = value.trim().to_string();
            if !normalized.is_empty() {
                profile.description = Some(normalized);
            }
        }
        if let Some(value) = preferred_alias {
            let normalized = value.trim().to_string();
            if !normalized.is_empty() {
                profile.preferred_alias = Some(normalized);
            }
        }
        profile.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn set_recovery_secret_for_did(&self, did_raw: &str, secret: &str) -> Result<()> {
        let did = Did::try_from(did_raw)
            .map_err(|e| anyhow!("invalid DID '{}': {}", did_raw, e))?;
        let did_root = did.without_fragment().id();
        let checksum = Self::recovery_checksum(secret);
        let mut profiles = self.closet_profiles.write().await;
        let profile = profiles.entry(did_root).or_default();
        profile.recovery_checksum = Some(checksum);
        profile.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn rekey_identity_from_recovery(
        &self,
        handle_raw: &str,
        recovery_secret: &str,
        new_did_raw: &str,
    ) -> Result<(String, String, String)> {
        let handle = handle_raw.trim().trim_start_matches('@').to_string();
        if handle.is_empty() {
            return Err(anyhow!("handle cannot be empty"));
        }
        let secret = recovery_secret.trim();
        if secret.is_empty() {
            return Err(anyhow!("recovery secret cannot be empty"));
        }

        let new_did = Did::try_from(new_did_raw)
            .map_err(|e| anyhow!("invalid new DID '{}': {}", new_did_raw, e))?;
        let new_root = new_did.without_fragment().id();

        let old_root = {
            let handle_to_did = self.handle_to_did.read().await;
            handle_to_did
                .get(&handle)
                .cloned()
                .ok_or_else(|| anyhow!("unknown avatar handle '{}'; expected existing handle", handle))?
        };

        let expected_checksum = {
            let profiles = self.closet_profiles.read().await;
            profiles
                .get(&old_root)
                .and_then(|p| p.recovery_checksum.clone())
                .ok_or_else(|| anyhow!("no recovery checksum is set for @{}", handle))?
        };
        if expected_checksum != Self::recovery_checksum(secret) {
            return Err(anyhow!("invalid recovery secret"));
        }

        {
            let did_to_handle = self.did_to_handle.read().await;
            if let Some(existing_handle) = did_to_handle.get(&new_root) {
                if existing_handle != &handle {
                    return Err(anyhow!(
                        "new DID already owns handle @{}; choose another DID",
                        existing_handle
                    ));
                }
            }
        }

        {
            let mut handle_to_did = self.handle_to_did.write().await;
            let current = handle_to_did
                .get(&handle)
                .cloned()
                .ok_or_else(|| anyhow!("handle mapping disappeared for @{}", handle))?;
            if current != old_root {
                return Err(anyhow!("handle @{} no longer maps to expected DID", handle));
            }
            handle_to_did.insert(handle.clone(), new_root.clone());
        }
        {
            let mut did_to_handle = self.did_to_handle.write().await;
            did_to_handle.remove(&old_root);
            did_to_handle.insert(new_root.clone(), handle.clone());
        }

        {
            let mut acl = self.entry_acl.write().await;
            acl.allowed_dids.remove(&old_root);
            acl.allowed_dids.insert(new_root.clone());
        }

        {
            let mut owner = self.owner_did.write().await;
            if owner.as_deref() == Some(old_root.as_str()) {
                *owner = Some(new_root.clone());
            }
        }

        {
            let mut rooms = self.rooms.write().await;
            for room in rooms.values_mut() {
                if room.acl.owner.as_deref() == Some(old_root.as_str()) {
                    room.acl.owner = Some(new_root.clone());
                }
                if room.acl.allow.remove(&old_root) {
                    room.acl.allow.insert(new_root.clone());
                }
                room.acl.deny.remove(&new_root);

                room.avatars
                    .retain(|_, av| av.agent_did.without_fragment().id() != old_root);
            }
        }

        {
            let mut room_objects = self.room_objects.write().await;
            for room_map in room_objects.values_mut() {
                for object in room_map.values_mut() {
                    if object.owner_did.as_deref() == Some(old_root.as_str()) {
                        object.owner_did = Some(new_root.clone());
                        object.meta_dirty = true;
                    }
                }
            }
        }

        {
            let mut profiles = self.closet_profiles.write().await;
            let mut profile = profiles.remove(&old_root).unwrap_or_default();
            profile.updated_at = Utc::now().to_rfc3339();
            profiles.insert(new_root.clone(), profile);
        }

        self.record_room_event(
            DEFAULT_ROOM,
            "system",
            Some(handle.clone()),
            Some(new_root.clone()),
            None,
            format!("{} rekeyed identity", handle),
        )
        .await;

        Ok((handle, old_root, new_root))
    }

    async fn did_root_by_endpoint(&self, room_name: &str, endpoint: &str) -> Option<String> {
        let rooms = self.rooms.read().await;
        let room = rooms.get(room_name)?;
        room.avatars
            .values()
            .find(|avatar| avatar.agent_endpoint == endpoint)
            .map(|avatar| avatar.agent_did.without_fragment().id())
    }

    async fn set_avatar_description_for_did(
        &self,
        room_name: &str,
        did_root: &str,
        description: &str,
    ) -> bool {
        let mut rooms = self.rooms.write().await;
        let Some(room) = rooms.get_mut(room_name) else {
            return false;
        };

        let mut updated = false;
        for avatar in room.avatars.values_mut() {
            if avatar.agent_did.without_fragment().id() == did_root {
                avatar.set_description(description.to_string());
                updated = true;
            }
        }
        updated
    }

    async fn avatar_language_order_for_did(&self, room_name: &str, did_root: &str) -> Option<String> {
        let rooms = self.rooms.read().await;
        let room = rooms.get(room_name)?;
        room.avatars
            .values()
            .find(|avatar| avatar.agent_did.without_fragment().id() == did_root)
            .map(|avatar| avatar.language_order.clone())
            .filter(|value| !value.trim().is_empty())
    }

    async fn closet_submit_citizenship(
        &self,
        session_id: &str,
        endpoint: &str,
        ipns_private_key_base64: &str,
        desired_alias: Option<&str>,
    ) -> Result<(String, String, String)> {
        let session = self.closet_session_owned_by(session_id, endpoint).await?;
        let (did, fragment, key_name) = if ipns_private_key_base64.trim().is_empty() {
            self.issue_bootstrap_did(endpoint, desired_alias).await?
        } else {
            self.issue_bootstrap_citizenship(endpoint, ipns_private_key_base64, desired_alias)
                .await?
        };

        self.upsert_closet_profile(
            &did,
            session.name.clone(),
            session.description.clone(),
            session.alias.clone(),
        )
            .await?;

        {
            let mut sessions = self.closet_sessions.write().await;
            if let Some(active) = sessions.get_mut(session_id) {
                active.did = Some(did.clone());
                active.fragment = Some(fragment.clone());
            }
        }

        Ok((did, fragment, key_name))
    }

    async fn closet_publish_did_document(
        &self,
        session_id: &str,
        endpoint: &str,
        did_document_json: &str,
        ipns_private_key_base64: &str,
        desired_fragment: Option<&str>,
    ) -> Result<(String, String, String)> {
        let session = self.closet_session_owned_by(session_id, endpoint).await?;
        let document = Document::unmarshal(did_document_json)
            .map_err(|e| anyhow!("invalid DID document JSON: {}", e))?;
        document
            .validate()
            .map_err(|e| anyhow!("invalid DID document: {}", e))?;
        document
            .verify()
            .map_err(|e| anyhow!("DID document signature verification failed: {}", e))?;

        let document_did = Did::try_from(document.id.as_str())
            .map_err(|e| anyhow!("invalid document DID '{}': {}", document.id, e))?
            ;
        let document_root = document_did.without_fragment().id();
        let document_ipns_id = document_did.ipns.clone();
        let provided_key = ipns_private_key_base64.trim();
        let kubo_url = self.kubo_url.read().await.clone();

        ensure_session_document_root_match(session.did.as_deref(), &document_root)?;

        let ipns_path = format!("/ipns/{}", document_ipns_id);
        let was_published_before = if provided_key.is_empty() {
            Some(kubo::name_resolve(&kubo_url, &ipns_path, true).await.is_ok())
        } else {
            None
        };

        let local_key_name = if provided_key.is_empty() {
            let keys = list_kubo_keys(&kubo_url).await?;
            keys.into_iter()
                .find(|key| key.id.trim() == document_ipns_id)
                .map(|key| key.name)
        } else {
            None
        };

        let publish_plan = plan_closet_did_publish(
            provided_key,
            &document_root,
            was_published_before,
            local_key_name,
        )?;

        let key_name = match publish_plan {
            ClosetDidPublishPlan::ImportProvidedKey => {
                let desired_alias = desired_fragment
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .or(document_did.fragment.as_deref());
                let (issued_did, _fragment, key_name) = self
                    .closet_submit_citizenship(session_id, endpoint, provided_key, desired_alias)
                    .await?;
                ensure_issued_document_root_match(&issued_did, &document_root)?;
                key_name
            }
            ClosetDidPublishPlan::UseExistingLocalKey { key_name } => {
                let mut sessions = self.closet_sessions.write().await;
                if let Some(active) = sessions.get_mut(session_id) {
                    active.did = Some(document_root.clone());
                    active.fragment = Some(key_name.clone());
                }
                key_name
            }
        };

        let document_cid = ipfs_add(&kubo_url, did_document_json.as_bytes().to_vec()).await?;
        let ipns_options = IpnsPublishOptions::default();
        ipns_publish_with_retry(
            &kubo_url,
            &key_name,
            &document_cid,
            &ipns_options,
            3,
            Duration::from_millis(1_000),
        )
        .await?;

        Ok((document_root, document_cid, key_name))
    }

    async fn closet_command(
        &self,
        session_id: &str,
        endpoint: &str,
        input: &str,
    ) -> Result<ClosetResponse> {
        let active_lang = self.closet_lang_for_session(session_id, endpoint).await;
        let command = parse_closet_command(input);

        if matches!(&command, ClosetCommand::Empty) {
            return Ok(ClosetResponse::ok(
                session_id,
                tr_world(active_lang, "closet.empty", "You are in the closet and have no avatar yet. Type 'help'."),
            ));
        }

        if matches!(&command, ClosetCommand::Help) {
            return Ok(
                ClosetResponse::ok(
                    session_id,
                    tr_world(active_lang, "closet.help", "Closet commands: help | show | hear | apply [ipns_key_base64] | citizen [ipns_key_base64] | avatar.help | avatar.peek | avatar.apply [ipns_key_base64] | avatar.name: <text> | avatar.description: <text> | avatar.name peek | avatar.description peek | document.help | document.peek | document.id peek | document.ma.transports peek | document.publish [ipns_key_base64] | document.republish [ipns_key_base64] | document.apply [ipns_key_base64] | recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>"),
                )
                .with_prompt(tr_world(active_lang, "closet.help.prompt", "If actor DID does not exist yet: run apply first. After actor is created, set avatar name/description. Then type 'go out' in the actor UI.")),
            );
        }

        if matches!(&command, ClosetCommand::Show) {
            let session = self.closet_session_owned_by(session_id, endpoint).await?;
            let did = session.did.clone().unwrap_or_else(|| "(none)".to_string());
            let name = session.name.clone().unwrap_or_else(|| "(unset)".to_string());
            let description = session
                .description
                .clone()
                .unwrap_or_else(|| "(unset)".to_string());
            let recovery = if let Some(did_raw) = session.did.as_deref() {
                let did_root = Did::try_from(did_raw)
                    .map_err(|e| anyhow!("invalid closet DID '{}': {}", did_raw, e))?
                    .without_fragment()
                    .id();
                self.closet_profiles
                    .read()
                    .await
                    .get(&did_root)
                    .and_then(|profile| profile.recovery_checksum.clone())
                    .is_some()
            } else {
                false
            };

            return Ok(
                ClosetResponse::ok(
                    session_id,
                    format!(
                        "closet profile: did={} name={} description={} recovery={}",
                        did,
                        name,
                        description,
                        if recovery { "set" } else { "unset" }
                    ),
                )
                .with_did_opt(session.did)
                .with_fragment_opt(session.fragment),
            );
        }

        if matches!(&command, ClosetCommand::Hear) {
            let since = {
                let sessions = self.closet_sessions.read().await;
                let Some(session) = sessions.get(session_id) else {
                    return Err(anyhow!("unknown closet session"));
                };
                if session.endpoint != endpoint {
                    return Err(anyhow!("closet session endpoint mismatch"));
                }
                session.last_lobby_sequence
            };

            let (events, latest_lobby_sequence) = self.room_events_since(DEFAULT_ROOM, since).await?;
            {
                let mut sessions = self.closet_sessions.write().await;
                if let Some(session) = sessions.get_mut(session_id) {
                    session.last_lobby_sequence = latest_lobby_sequence;
                }
            }

            return Ok(
                ClosetResponse::ok(
                    session_id,
                    if events.is_empty() {
                        tr_world(active_lang, "closet.hear.none", "No new lobby events.")
                    } else {
                        tr_world_vars(
                            active_lang,
                            "closet.hear.count",
                            &[("count", events.len().to_string())],
                            &format!("Heard {} lobby event(s).", events.len()),
                        )
                    },
                )
                .with_lobby_events(events, latest_lobby_sequence),
            );
        }

        match command {
            ClosetCommand::MissingFieldValue { field } => {
                return Err(anyhow!("usage: {} <value>", field.as_str()));
            }
            ClosetCommand::SetField { field, value } => {
                let session = self.closet_session_owned_by(session_id, endpoint).await?;
                if session.did.is_none() {
                    return Ok(
                        ClosetResponse::err(
                            session_id,
                            tr_world(
                                active_lang,
                                "closet.actor.required",
                                "actor identity does not exist in this world yet; run apply first",
                            ),
                        )
                        .with_prompt(tr_world(
                            active_lang,
                            "closet.actor.required.prompt",
                            "run apply first; after actor creation you can set name/description",
                        )),
                    );
                }

                let msg = self
                    .closet_answer(session_id, endpoint, field.as_str(), value.as_str())
                    .await?;

                if let Some(did) = session.did.as_deref() {
                    self.upsert_closet_profile(
                        did,
                        session.name.clone(),
                        session.description.clone(),
                        session.alias.clone(),
                    )
                    .await?;

                    if matches!(field, ClosetProfileField::Description) {
                        let did_root = Did::try_from(did)
                            .map_err(|e| anyhow!("invalid closet DID '{}': {}", did, e))?
                            .without_fragment()
                            .id();
                        let _ = self
                            .set_avatar_description_for_did(
                                DEFAULT_ROOM,
                                did_root.as_str(),
                                value.trim(),
                            )
                            .await;
                    }
                }

                return Ok(ClosetResponse::ok(session_id, msg));
            }
            ClosetCommand::Apply {
                ipns_private_key_base64,
            } => {
                let session = self.closet_session_owned_by(session_id, endpoint).await?;

                if let Some(existing_did) = session.did.as_deref() {
                    self.upsert_closet_profile(
                        existing_did,
                        session.name.clone(),
                        session.description.clone(),
                        session.alias.clone(),
                    )
                    .await?;
                    return Ok(
                        ClosetResponse::ok(
                            session_id,
                            tr_world(
                                active_lang,
                                "closet.apply.updated",
                                "changes applied. You can stay in the closet and keep editing; type 'go out' when ready.",
                            ),
                        )
                        .with_did(existing_did.to_string())
                        .with_fragment_opt(session.fragment),
                    );
                }

                let (did, fragment, key_name) = self
                    .closet_submit_citizenship(
                        session_id,
                        endpoint,
                        ipns_private_key_base64.as_str(),
                        session.alias.as_deref(),
                    )
                    .await?;
                return Ok(
                    ClosetResponse::ok(
                        session_id,
                        tr_world(
                            active_lang,
                            "closet.apply.accepted",
                            "application accepted. type 'go out' in the actor UI to enter the world.",
                        ),
                    )
                    .with_did(did)
                    .with_fragment(fragment)
                    .with_key_name(key_name),
                );
            }
            ClosetCommand::Recovery(recovery_command) => {
                match recovery_command {
                    ClosetRecoveryCommand::Usage => {
                        return Ok(ClosetResponse::err(
                            session_id,
                            tr_world(
                                active_lang,
                                "closet.recovery.usage",
                                "usage: recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>",
                            ),
                        ));
                    }
                    ClosetRecoveryCommand::Set { passphrase } => {
                        if passphrase.len() < 8 {
                            return Ok(ClosetResponse::err(
                                session_id,
                                tr_world(
                                    active_lang,
                                    "closet.recovery.passphrase.short",
                                    "recovery passphrase must be at least 8 characters",
                                ),
                            ));
                        }

                        let session = self.closet_session_owned_by(session_id, endpoint).await?;
                        let did = if let Some(did) = session.did.clone() {
                            did
                        } else if let Some(root) = self.did_root_by_endpoint(DEFAULT_ROOM, endpoint).await {
                            root
                        } else {
                            return Ok(ClosetResponse::err(
                                session_id,
                                tr_world(
                                    active_lang,
                                    "closet.recovery.set.requires_did",
                                    "recovery set requires a DID in this closet session (run apply first or open closet while logged in)",
                                ),
                            ));
                        };

                        self.set_recovery_secret_for_did(&did, passphrase.as_str()).await?;
                        return Ok(ClosetResponse::ok(
                            session_id,
                            tr_world(
                                active_lang,
                                "closet.recovery.set.stored",
                                "recovery checksum stored",
                            ),
                        ));
                    }
                    ClosetRecoveryCommand::Status => {
                        let session = self.closet_session_owned_by(session_id, endpoint).await?;
                        let did = if let Some(did) = session.did.clone() {
                            Some(did)
                        } else {
                            self.did_root_by_endpoint(DEFAULT_ROOM, endpoint).await
                        };
                        let Some(did) = did else {
                            return Ok(ClosetResponse::err(
                                session_id,
                                tr_world(
                                    active_lang,
                                    "closet.recovery.status.no_context",
                                    "no DID context for this closet session",
                                ),
                            ));
                        };
                        let did_root = Did::try_from(did.as_str())
                            .map_err(|e| anyhow!("invalid DID '{}': {}", did, e))?
                            .without_fragment()
                            .id();
                        let configured = self
                            .closet_profiles
                            .read()
                            .await
                            .get(&did_root)
                            .and_then(|profile| profile.recovery_checksum.clone())
                            .is_some();

                        return Ok(ClosetResponse::ok(
                            session_id,
                            if configured {
                                tr_world(
                                    active_lang,
                                    "closet.recovery.status.configured",
                                    "recovery is configured",
                                )
                            } else {
                                tr_world(
                                    active_lang,
                                    "closet.recovery.status.not_configured",
                                    "recovery is not configured",
                                )
                            },
                        ));
                    }
                    ClosetRecoveryCommand::Rekey { handle, passphrase } => {
                        if handle.is_empty() || passphrase.is_empty() {
                            return Ok(ClosetResponse::err(
                                session_id,
                                tr_world(
                                    active_lang,
                                    "closet.recovery.rekey.usage",
                                    "usage: recovery rekey <@handle> <passphrase>",
                                ),
                            ));
                        }

                        let session = self.closet_session_owned_by(session_id, endpoint).await?;
                        let Some(new_did) = session.did.clone() else {
                            return Ok(ClosetResponse::err(
                                session_id,
                                tr_world(
                                    active_lang,
                                    "closet.recovery.rekey.requires_new_did",
                                    "closet_recovery_rekey_requires_new_did: run apply first",
                                ),
                            ));
                        };

                        let (resolved_handle, old_root, new_root) = self
                            .rekey_identity_from_recovery(handle.as_str(), passphrase.as_str(), &new_did)
                            .await?;
                        return Ok(
                            ClosetResponse::ok(
                                session_id,
                                tr_world_vars(
                                    active_lang,
                                    "closet.recovery.rekey.done",
                                    &[
                                        ("handle", resolved_handle.clone()),
                                        ("old", old_root.clone()),
                                        ("new", new_root.clone()),
                                    ],
                                    &format!(
                                        "rekey complete for @{} ({} -> {})",
                                        resolved_handle, old_root, new_root
                                    ),
                                ),
                            )
                            .with_did(new_did),
                        );
                    }
                }
            }
            ClosetCommand::Enter { room } => {
                let (did, preferred_handle, session_fragment) = {
                    let sessions = self.closet_sessions.read().await;
                    let Some(session) = sessions.get(session_id) else {
                        return Err(anyhow!("unknown closet session"));
                    };
                    if session.endpoint != endpoint {
                        return Err(anyhow!("closet session endpoint mismatch"));
                    }
                    let did = session
                        .did
                        .clone()
                        .ok_or_else(|| anyhow!("no DID in session; run 'apply' first"))?;
                    (did, session.name.clone(), session.fragment.clone())
                };

                let room = room
                    .map(|value| {
                        let trimmed = value.trim();
                        if trimmed.eq_ignore_ascii_case("out") {
                            DEFAULT_ROOM.to_string()
                        } else {
                            trimmed.to_string()
                        }
                    })
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| DEFAULT_ROOM.to_string());

                let did_obj = Did::try_from(did.as_str())
                    .map_err(|e| anyhow!("invalid session DID '{}': {}", did, e))?;
                let root_did = did_obj.without_fragment();

                if !self.can_enter(&root_did).await {
                    let did_root = root_did.id();
                    let (knock_id, duplicate) = self
                        .enqueue_knock(&room, &did_root, endpoint, preferred_handle)
                        .await;
                    let detail = if duplicate {
                        format!(
                            "entry denied for {}; existing knock request id={} is pending",
                            did_root, knock_id
                        )
                    } else {
                        format!(
                            "entry denied for {}; knock request queued with id={}",
                            did_root, knock_id
                        )
                    };
                    return Ok(ClosetResponse::err(session_id, detail).with_did(did_root));
                }

                let inbox = resolve_avatar_inbox_from_session(&did_obj, session_fragment.as_deref())?;
                let avatar_req = AvatarRequest {
                    inbox,
                    did: root_did,
                    agent_endpoint: endpoint.to_string(),
                    language_order: "nb_NO:en_UK".to_string(),
                };
                let handle = self.join_room(&room, avatar_req, preferred_handle).await?;

                return Ok(
                    ClosetResponse::ok(
                        session_id,
                        tr_world_vars(
                            active_lang,
                            "closet.entered",
                            &[("room", room.clone()), ("handle", handle.clone())],
                            &format!("entered {} as @{}", room, handle),
                        ),
                    )
                    .with_did(did),
                );
            }
            ClosetCommand::Unknown { verb } => {
                return Ok(ClosetResponse::err(
                    session_id,
                    tr_world_vars(
                        active_lang,
                        "closet.command.unknown",
                        &[("verb", verb.clone())],
                        &format!("Unknown closet command '{}'. Type 'help'.", verb),
                    ),
                ));
            }
            ClosetCommand::Empty
            | ClosetCommand::Help
            | ClosetCommand::Show
            | ClosetCommand::Hear => {}
        }

        unreachable!("closet command handling returned no response")
    }

    async fn issue_bootstrap_citizenship(
        &self,
        requester_endpoint: &str,
        ipns_private_key_base64: &str,
        desired_alias: Option<&str>,
    ) -> Result<(String, String, String)> {
        let kubo_url = self.kubo_url.read().await.clone();
        let existing_names = list_kubo_key_names(&kubo_url).await?;

        let fragment = if let Some(alias) = desired_alias {
            let normalized = alias.trim();
            if normalized.is_empty() {
                return Err(anyhow!("alias cannot be empty"));
            }
            if !is_valid_nanoid_id(normalized) {
                return Err(anyhow!(
                    "invalid alias '{}': expected [A-Za-z0-9_-]+",
                    normalized
                ));
            }
            if existing_names.iter().any(|name| name == normalized) {
                return Err(anyhow!(
                    "alias '{}' is already taken (username/fragment in use); choose another alias and run apply again",
                    normalized
                ));
            }
            normalized.to_string()
        } else {
            let mut generated: Option<String> = None;
            for _ in 0..64 {
                let candidate = nanoid!();
                if !existing_names.iter().any(|name| name == &candidate) {
                    generated = Some(candidate);
                    break;
                }
            }
            generated.ok_or_else(|| anyhow!("failed to allocate a unique fragment"))?
        };

        let key_bytes = B64
            .decode(ipns_private_key_base64.trim().as_bytes())
            .map_err(|e| anyhow!("invalid ipns key base64: {}", e))?;
        if key_bytes.is_empty() {
            return Err(anyhow!("ipns key payload is empty"));
        }

        let imported = import_kubo_key(&kubo_url, &fragment, key_bytes).await?;
        let did = Did::new_root(&imported.id)
            .map_err(|e| anyhow!("failed to construct did from key id '{}': {}", imported.id, e))?
            .id();

        let did_obj = Did::try_from(did.as_str())
            .map_err(|e| anyhow!("failed to parse imported did '{}': {}", did, e))?;
        if !self.can_enter(&did_obj).await {
            let _ = remove_kubo_key(&kubo_url, &imported.name).await;
            let did_root = did_obj.without_fragment().id();
            let (knock_id, duplicate) = self
                .enqueue_knock(DEFAULT_ROOM, &did_root, requester_endpoint, None)
                .await;
            let message = if duplicate {
                format!(
                    "entry denied for {}; existing knock request id={} is pending",
                    did_root, knock_id
                )
            } else {
                format!(
                    "entry denied for {}; knock request queued with id={}",
                    did_root, knock_id
                )
            };
            return Err(anyhow!(message));
        }

        Ok((did, fragment, imported.name))
    }

    async fn ensure_lobby_intrinsic_objects(&self) {
        let room_name = DEFAULT_ROOM;
        let rooms = self.rooms.read().await;
        if !rooms.contains_key(room_name) {
            return;
        }
        drop(rooms);

        let mut objects = self.room_objects.write().await;
        let room_map = objects
            .entry(room_name.to_string())
            .or_insert_with(HashMap::new);
        room_map.entry("mailbox".to_string()).or_insert_with(|| {
            let mailbox = ObjectRuntimeState::intrinsic_mailbox(room_name);
            if let Some(definition) = mailbox.definition.as_ref() {
                if let Err(err) = content_validation::validate_object_definition(definition, "intrinsic:mailbox") {
                    warn!("invalid intrinsic mailbox definition: {}", err);
                }
            }
            mailbox
        });
        room_map.entry("closet".to_string()).or_insert_with(|| {
            let closet = ObjectRuntimeState::intrinsic_closet(room_name);
            if let Some(definition) = closet.definition.as_ref() {
                if let Err(err) = content_validation::validate_object_definition(definition, "intrinsic:closet") {
                    warn!("invalid intrinsic closet definition: {}", err);
                }
            }
            closet
        });
    }

    async fn find_intrinsic_mailbox_location(&self) -> Option<(String, String)> {
        let objects = self.room_objects.read().await;
        for (room_id, room_map) in objects.iter() {
            if let Some((object_id, _)) = room_map
                .iter()
                .find(|(_, object)| {
                    object.has_receiver_role("world-inbox")
                        || object.has_receiver_protocol("ma/inbox/1")
                })
            {
                return Some((room_id.clone(), object_id.clone()));
            }
        }
        None
    }

    async fn room_object_names(&self, room_name: &str) -> Vec<String> {
        let objects = self.room_objects.read().await;
        let Some(room_map) = objects.get(room_name) else {
            return Vec::new();
        };
        room_map.values().map(|obj| obj.name.clone()).collect()
    }

    async fn room_object_did_map(&self, room_name: &str) -> HashMap<String, String> {
        let world_root = self
            .local_world_did_root()
            .await
            .unwrap_or_else(|| "did:ma:unconfigured".to_string());
        let objects = self.room_objects.read().await;
        let Some(room_map) = objects.get(room_name) else {
            return HashMap::new();
        };

        let mut out = HashMap::new();
        for object in room_map.values() {
            let object_did = format!("{}#{}", world_root, object.id);
            out.insert(object.id.to_ascii_lowercase(), object_did.clone());
            out.insert(object.name.to_ascii_lowercase(), object_did.clone());
            for alias in &object.aliases {
                let token = alias.trim().trim_start_matches('@').to_ascii_lowercase();
                if !token.is_empty() {
                    out.insert(token, object_did.clone());
                }
            }
        }
        out
    }

    async fn resolve_room_object_id(&self, room_name: &str, target: &str) -> Option<String> {
        let raw = target.trim();
        if raw.is_empty() {
            return None;
        }

        if let Ok(did) = Did::try_from(raw) {
            let root = did.without_fragment().id();
            if !self.is_local_world_root(&root).await {
                return None;
            }
            if let Some(fragment) = did.fragment.clone() {
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                if room_map.contains_key(&fragment) {
                    return Some(fragment);
                }
            }
            return None;
        }

        let lookup = raw.trim_start_matches('@');
        let objects = self.room_objects.read().await;
        let room_map = objects.get(room_name)?;
        room_map
            .values()
            .find(|obj| obj.matches_target(lookup))
            .map(|obj| obj.id.clone())
    }

    async fn enqueue_object_durable_inbox_message(
        &self,
        room_name: &str,
        object_id: &str,
        message: ObjectInboxMessage,
    ) -> bool {
        let mut objects = self.room_objects.write().await;
        let Some(room_map) = objects.get_mut(room_name) else {
            return false;
        };
        let Some(object) = room_map.get_mut(object_id) else {
            return false;
        };
        object.push_durable_inbox_message(message, MAX_OBJECT_INBOX);
        true
    }

    #[allow(dead_code)]
    async fn enqueue_object_ephemeral_inbox_message(
        &self,
        room_name: &str,
        object_id: &str,
        message: ObjectInboxMessage,
    ) -> bool {
        let mut objects = self.room_objects.write().await;
        let Some(room_map) = objects.get_mut(room_name) else {
            return false;
        };
        let Some(object) = room_map.get_mut(object_id) else {
            return false;
        };
        object.push_ephemeral_inbox_message(message, MAX_OBJECT_INBOX);
        true
    }

    #[allow(dead_code)]
    async fn pop_object_inbox_message(
        &self,
        room_name: &str,
        object_id: &str,
    ) -> Option<ObjectInboxMessage> {
        let mut objects = self.room_objects.write().await;
        let room_map = objects.get_mut(room_name)?;
        let object = room_map.get_mut(object_id)?;
        object.pop_inbox_message()
    }

    #[allow(dead_code)]
    async fn queue_object_outbound_intent(
        &self,
        room_name: &str,
        object_id: &str,
        intent: ObjectMessageIntent,
    ) -> bool {
        let mut objects = self.room_objects.write().await;
        let Some(room_map) = objects.get_mut(room_name) else {
            return false;
        };
        let Some(object) = room_map.get_mut(object_id) else {
            return false;
        };
        object.queue_outbound_intent(intent);
        true
    }

    async fn load_global_capability_acl_from_cid(&self, acl_cid: &str) -> Result<()> {
        let compiled = self.load_compiled_acl_from_cid_cached(acl_cid).await?;
        *self.global_capability_acl.write().await = Some(compiled);
        *self.global_capability_acl_source.write().await = Some(acl_cid.to_string());
        Ok(())
    }

    async fn load_compiled_acl_from_cid_cached(&self, acl_cid: &str) -> Result<CompiledCapabilityAcl> {
        if let Some(cached) = self.capability_acl_cache.read().await.get(acl_cid).cloned() {
            return Ok(cached);
        }

        let kubo_url = self.kubo_url().await;
        let raw = kubo::cat_cid(&kubo_url, acl_cid)
            .await
            .map_err(|e| anyhow!("failed loading capability ACL {}: {}", acl_cid, e))?;
        let acl = parse_capability_acl_text(&raw, acl_cid)?;
        let compiled = compile_acl(&acl, acl_cid)?;

        self.capability_acl_cache
            .write()
            .await
            .insert(acl_cid.to_string(), compiled.clone());

        Ok(compiled)
    }

    async fn object_capability_allowed(
        &self,
        room_name: &str,
        object_id: &str,
        caller_root_did: &str,
        capability: &str,
    ) -> Result<bool> {
        let (object_owner, object_state) = {
            let objects = self.room_objects.read().await;
            let Some(room_map) = objects.get(room_name) else {
                return Ok(false);
            };
            let Some(object) = room_map.get(object_id) else {
                return Ok(false);
            };
            (object.owner_did.clone(), object.state.clone())
        };

        let world_owner = self.owner_did.read().await.clone();

        let global_match = {
            let global_acl = self.global_capability_acl.read().await;
            match global_acl.as_ref() {
                None => true,
                Some(acl) => evaluate_compiled_acl_with_owner(
                    acl,
                    caller_root_did,
                    world_owner.as_deref(),
                    capability,
                ),
            }
        };
        if !global_match {
            return Ok(false);
        }

        let local_acl_cid = object_state
            .as_object()
            .and_then(|obj| {
                obj.get("acl_cid")
                    .or_else(|| obj.get("capabilities_acl_cid"))
            })
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|cid| !cid.is_empty())
            .map(str::to_string);

        let local_match = if let Some(acl_cid) = local_acl_cid {
            let compiled_local = self.load_compiled_acl_from_cid_cached(&acl_cid).await?;
            evaluate_compiled_acl_with_owner(
                &compiled_local,
                caller_root_did,
                object_owner.as_deref(),
                capability,
            )
        } else {
            let local_acl = parse_object_local_capability_acl(&object_state)?;
            match local_acl.as_ref() {
                None => true,
                Some(acl) => {
                    let compiled_local = compile_acl(acl, "object-local-acl")?;
                    evaluate_compiled_acl_with_owner(
                        &compiled_local,
                        caller_root_did,
                        object_owner.as_deref(),
                        capability,
                    )
                }
            }
        };

        Ok(local_match)
    }

    pub async fn kubo_url(&self) -> String {
        self.kubo_url.read().await.clone()
    }

    pub async fn set_kubo_url(&self, new_url: &str) -> Result<String> {
        let trimmed = new_url.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("kubo api url cannot be empty"));
        }
        *self.kubo_url.write().await = trimmed.to_string();
        Ok(trimmed.to_string())
    }

    pub async fn set_world_root_pin_name(&self, new_slug: &str) -> Result<String> {
        let normalized = normalize_world_key_name(new_slug);
        *self.world_root_pin_name.write().await = normalized.clone();
        let kubo_url = self.kubo_url().await;

        if let Some(current_cid) = self.world_cid.read().await.clone() {
            // Re-attach current world root CID with the new name.
            pin_add_named(&kubo_url, &current_cid, &normalized).await?;
        }

        Ok(normalized)
    }

    pub async fn set_unlock_context(&self, world_dir: PathBuf, world_master_key_path: PathBuf) {
        *self.unlock_world_dir.write().await = Some(world_dir);
        *self.world_master_key_path.write().await = Some(world_master_key_path);
    }

    pub async fn set_world_master_key(&self, world_master_key: [u8; 32]) {
        *self.unlocked_world_master_key.write().await = Some(world_master_key);
        *self.unlocked_world_signing_key.write().await =
            Some(derive_world_signing_private_key(&world_master_key));
        *self.unlocked_world_encryption_key.write().await =
            Some(derive_world_encryption_private_key(&world_master_key));
    }

    pub async fn lock(&self) {
        *self.unlocked.write().await = false;
    }

    pub async fn is_unlocked(&self) -> bool {
        *self.unlocked.read().await
    }

    pub async fn create_unlock_bundle(&self, passphrase: &str) -> Result<String> {
        let passphrase = passphrase.trim();
        if passphrase.len() < 8 {
            return Err(anyhow!("passphrase must be at least 8 characters"));
        }
        let world_master_key = self.read_world_master_key().await?;
        let plain = WorldAccessBundlePlain {
            version: 2,
            world_master_key_b64: B64.encode(world_master_key),
            world_signing_private_key_b64: None,
            world_encryption_private_key_b64: None,
        };
        let plain_bytes = serde_json::to_vec(&plain)
            .map_err(|e| anyhow!("failed to encode bundle payload: {}", e))?;

        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut nonce);

        let bundle_key = derive_bundle_key_argon2(passphrase.as_bytes(), &salt)?;
        let cipher = XChaCha20Poly1305::new((&bundle_key).into());
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), plain_bytes.as_ref())
            .map_err(|_| anyhow!("failed to encrypt unlock bundle"))?;

        let bundle = WorldAccessBundle {
            version: 1,
            kdf: "argon2id".to_string(),
            salt_b64: B64.encode(salt),
            nonce_b64: B64.encode(nonce),
            ciphertext_b64: B64.encode(ciphertext),
        };

        serde_json::to_string(&bundle)
            .map_err(|e| anyhow!("failed to serialize unlock bundle: {}", e))
    }

    pub async fn unlock_runtime(&self, passphrase: &str, bundle_json: &str) -> Result<usize> {
        if passphrase.trim().is_empty() {
            return Err(anyhow!("missing passphrase"));
        }
        if bundle_json.trim().is_empty() {
            return Err(anyhow!("missing bundle"));
        }

        let secrets = decrypt_world_access_bundle(passphrase, bundle_json)?;

        if let Some(path) = self.world_master_key_path.read().await.clone() {
            let file_bytes = fs::read(&path)
                .map_err(|e| anyhow!("failed reading world master key {}: {}", path.display(), e))?;
            let file_master_key: [u8; 32] = file_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("world master key must be 32 bytes in {}", path.display()))?;
            if file_master_key != secrets.world_master_key {
                return Err(anyhow!("bundle does not match this world"));
            }
        } else if let Some(runtime_master_key) = self.unlocked_world_master_key.read().await.clone() {
            if runtime_master_key != secrets.world_master_key {
                return Err(anyhow!("bundle does not match this world"));
            }
        }

        *self.unlocked_world_master_key.write().await = Some(secrets.world_master_key);
        *self.unlocked_world_signing_key.write().await = Some(secrets.world_signing_private_key);
        *self.unlocked_world_encryption_key.write().await = Some(secrets.world_encryption_private_key);

        let Some(world_dir) = self.unlock_world_dir.read().await.clone() else {
            *self.unlocked.write().await = true;
            return Ok(0);
        };

        let loaded = load_world_authoring(&world_dir)?;
        let bundles = unlock_actor_secret_bundles(&loaded)?;
        let count = bundles.len();
        self.install_actor_secrets(&bundles).await?;
        *self.unlocked.write().await = true;
        Ok(count)
    }

    async fn read_world_master_key(&self) -> Result<[u8; 32]> {
        if let Some(key) = self.unlocked_world_master_key.read().await.clone() {
            return Ok(key);
        }

        let Some(path) = self.world_master_key_path.read().await.clone() else {
            return Err(anyhow!("world master key path is not configured"));
        };

        let bytes = fs::read(&path)
            .map_err(|e| anyhow!("failed reading world master key {}: {}", path.display(), e))?;
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("world master key must be 32 bytes in {}", path.display()))
    }

    async fn read_world_runtime_secrets(&self) -> Result<WorldRuntimeSecrets> {
        if let (Some(master), Some(signing), Some(encryption)) = (
            self.unlocked_world_master_key.read().await.clone(),
            self.unlocked_world_signing_key.read().await.clone(),
            self.unlocked_world_encryption_key.read().await.clone(),
        ) {
            return Ok(WorldRuntimeSecrets {
                world_master_key: master,
                world_signing_private_key: signing,
                world_encryption_private_key: encryption,
            });
        }

        let master = self.read_world_master_key().await?;
        Ok(WorldRuntimeSecrets {
            world_master_key: master,
            world_signing_private_key: derive_world_signing_private_key(&master),
            world_encryption_private_key: derive_world_encryption_private_key(&master),
        })
    }

    pub async fn set_world_did_root(&self, world_did: &str) -> Result<()> {
        let root = Did::try_from(world_did)
            .map_err(|e| anyhow!("invalid world DID '{}': {}", world_did, e))?
            .without_fragment()
            .id();

        *self.world_did_root.write().await = Some(root.clone());
        *self.world_did.write().await = Some(world_did.to_string());

        // Keep runtime rooms aligned with the configured world DID root.
        // This fixes stale values like did:ma:unconfigured#lobby created before DID bootstrap.
        {
            let mut rooms = self.rooms.write().await;
            for (room_name, room) in rooms.iter_mut() {
                room.did = format!("{}#{}", root, room_name);
            }
        }

        // Bootstrap owner identity from the world DID root when owner has not
        // been explicitly restored yet (e.g. first boot or missing runtime state).
        // This keeps entry ACL policy-driven while avoiding owner lockout.
        let owner_missing = self.owner_did.read().await.is_none();
        if owner_missing {
            *self.owner_did.write().await = Some(root.clone());
            self.allow_entry_did(&root).await;
        }

        Ok(())
    }

    async fn local_world_did_root(&self) -> Option<String> {
        self.world_did_root.read().await.clone()
    }

    async fn build_room_did(&self, room_id: &str) -> String {
        let root = self
            .local_world_did_root()
            .await
            .unwrap_or_else(|| "did:ma:unconfigured".to_string());
        format!("{}#{}", root, room_id)
    }

    async fn materialize_room_from_yaml(&self, room_name: &str, room_yaml: &str) -> Result<(Room, bool)> {
        let kubo_url = self.kubo_url().await;
        let canonical_did = self.build_room_did(room_name).await;

        // Preferred format: room YAML v2 references exits/avatars by CID.
        if let Ok(doc) = serde_yaml::from_str::<RoomYamlDocV2>(room_yaml) {
            let authored_did = doc.did.unwrap_or_default().trim().to_string();
            let (room_did, needs_rewrite) = match Did::try_from(authored_did.as_str()) {
                Ok(_) => (authored_did, false),
                Err(_) => (canonical_did.clone(), true),
            };
            let mut room = Room::new(doc.id.clone(), room_did);
            room.titles = doc.titles;
            room.descriptions = doc.descriptions;

            let mut exits = Vec::new();
            if !doc.exit_cids.is_empty() {
                let mut exit_items = doc.exit_cids.into_iter().collect::<Vec<_>>();
                exit_items.sort_by(|a, b| a.0.cmp(&b.0));
                for (exit_id, cid) in exit_items {
                    match kubo::cat_cid(&kubo_url, &cid).await {
                        Ok(exit_yaml) => match serde_yaml::from_str::<ExitYamlDoc>(&exit_yaml) {
                            Ok(exit_doc) => exits.push(exit_doc.exit),
                            Err(err) => warn!(
                                "Failed decoding exit '{}' from {} in room '{}': {}",
                                exit_id,
                                cid,
                                room_name,
                                err
                            ),
                        },
                        Err(err) => warn!(
                            "Failed loading exit '{}' from {} in room '{}': {}",
                            exit_id,
                            cid,
                            room_name,
                            err
                        ),
                    }
                }
            } else if !doc.exits.is_empty() {
                // Backward compatibility: accept inline exits when no exit_cids are present.
                exits = doc.exits;
            }
            exits.sort_by(|a, b| a.name.cmp(&b.name));
            room.exits = exits;

            return Ok((room, needs_rewrite));
        }

        // Legacy format: embedded room YAML (name/title/exits/acl/descriptions/did).
        let legacy = serde_yaml::from_str::<LegacyRoomYaml>(room_yaml)
            .map_err(|e| anyhow!("invalid room YAML for '{}': {}", room_name, e))?;

        let room_id = if legacy.name.trim().is_empty() {
            room_name.to_string()
        } else {
            legacy.name
        };
        let authored_did = legacy.did.trim().to_string();
        let (room_did, needs_rewrite) = match Did::try_from(authored_did.as_str()) {
            Ok(_) => (authored_did, false),
            Err(_) => (canonical_did, true),
        };
        let mut room = Room::new(room_id, room_did);
        room.exits = legacy.exits;
        room.descriptions = legacy.descriptions;

        let title_value = legacy.title.trim().to_string();
        if !title_value.is_empty() {
            room.set_title(title_value);
        }

        // ACL/owner are runtime metadata and are not loaded from room CID definitions.

        Ok((room, needs_rewrite))
    }

    async fn is_local_world_root(&self, root: &str) -> bool {
        self.world_did_root
            .read()
            .await
            .as_ref()
            .map(|local| local == root)
            .unwrap_or(false)
    }

    async fn is_world_target_did(&self, target: &str) -> bool {
        let parsed = match Did::try_from(target) {
            Ok(did) => did,
            Err(_) => return false,
        };
        let target_full = parsed.id();
        let target_root = parsed.without_fragment().id();

        let configured_root = self.world_did_root.read().await.clone();
        let configured_full = self.world_did.read().await.clone();

        configured_root
            .as_ref()
            .map(|v| v == &target_root)
            .unwrap_or(false)
            || configured_full
                .as_ref()
                .map(|v| v == &target_full)
                .unwrap_or(false)
    }

    pub(crate) async fn install_actor_secrets(
        &self,
        bundles: &HashMap<String, ActorSecretBundle>,
    ) -> Result<()> {
        let mut decoded = HashMap::new();
        for (actor_id, bundle) in bundles {
            let signing_raw = B64
                .decode(&bundle.secrets.signing_key_b64)
                .map_err(|e| anyhow!("invalid signing key for {}: {}", actor_id, e))?;
            let signing_key: [u8; 32] = signing_raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("signing key for {} must be 32 bytes", actor_id))?;

            decoded.insert(
                actor_id.clone(),
                RuntimeActorSecret {
                    signing_key,
                },
            );
        }

        let mut slots = self.actor_secrets.write().await;
        *slots = decoded;
        Ok(())
    }

    pub async fn can_enter(&self, did: &Did) -> bool {
        let did_root = did.without_fragment().id();
        // Entry decisions are ACL-driven only.
        let acl = self.entry_acl.read().await;
        if acl.allow_all {
            return true;
        }
        if acl.allow_owner
            && self
                .owner_did
                .read()
                .await
                .as_ref()
                .is_some_and(|owner| owner == &did_root)
        {
            return true;
        }
        acl.allowed_dids.contains(&did_root)
    }

    pub async fn entry_acl_source(&self) -> String {
        self.entry_acl.read().await.source.clone()
    }

    pub async fn entry_acl_debug(&self) -> (bool, bool, usize, Option<String>, String) {
        let acl = self.entry_acl.read().await;
        let owner = self.owner_did.read().await.clone();
        (
            acl.allow_all,
            acl.allow_owner,
            acl.allowed_dids.len(),
            owner,
            acl.source.clone(),
        )
    }

    pub async fn is_private_world(&self) -> bool {
        !self.entry_acl.read().await.allow_all
    }

    pub async fn set_private_world(&self, enabled: bool) {
        let mut acl = self.entry_acl.write().await;
        acl.allow_all = !enabled;
        acl.allow_owner = true;
        if enabled {
            acl.source = "runtime:private".to_string();
        } else {
            acl.source = "runtime:public".to_string();
        }
    }

    pub async fn allow_entry_did(&self, did_root: &str) {
        let mut acl = self.entry_acl.write().await;
        acl.allowed_dids.insert(did_root.to_string());
        if acl.allow_all {
            acl.source = "runtime:public(+allowlist)".to_string();
        } else {
            acl.source = "runtime:private(+allowlist)".to_string();
        }
    }

    fn parse_knock_id_arg(id_raw: &str) -> Result<u64, String> {
        id_raw
            .parse::<u64>()
            .map_err(|_| format!("invalid knock id '{}'", id_raw))
    }

    fn parse_invite_root_did_arg(target_did_raw: &str) -> Result<String, String> {
        Did::try_from(target_did_raw)
            .map(|did| did.without_fragment().id())
            .map_err(|err| format!("invalid DID '{}': {}", target_did_raw, err))
    }

    fn lookup_object_print_verb(
        object: &ObjectRuntimeState,
        verb: &str,
        _sender_profile: &str,
    ) -> Option<String> {
        let verbs = object.definition.as_ref().map(|def| &def.verbs)?;

        let needle = verb.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }

        for entry in verbs {
            let name_matches = entry.name.trim().eq_ignore_ascii_case(needle.as_str());
            let alias_matches = entry
                .aliases
                .iter()
                .any(|value| value.trim().eq_ignore_ascii_case(needle.as_str()));

            if !name_matches && !alias_matches {
                continue;
            }

            let evaluator_name = entry.evaluator.name.trim().to_ascii_lowercase();
            let evaluator_type = entry.evaluator.evaluator_type.trim().to_ascii_lowercase();
            let evaluator_ok = (evaluator_type == "built-in" || evaluator_type == "builtin")
                && matches!(evaluator_name.as_str(), "print" | "output" | "printf" | "format");

            if !evaluator_ok {
                continue;
            }

            let Some(content) = entry.content.clone() else {
                continue;
            };

            return Some(content);
        }

        None
    }

    fn lookup_object_verb_definition(
        object: &ObjectRuntimeState,
        verb: &str,
    ) -> Option<ma_core::ObjectVerbDefinition> {
        let verbs = object.definition.as_ref().map(|def| &def.verbs)?;
        let needle = verb.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }

        verbs
            .iter()
            .find(|entry| {
                entry.name.trim().eq_ignore_ascii_case(needle.as_str())
                    || entry
                        .aliases
                        .iter()
                        .any(|value| value.trim().eq_ignore_ascii_case(needle.as_str()))
            })
            .cloned()
    }

    fn parse_object_definition_text(raw: &str, cid: &str) -> Result<ObjectDefinition> {
        content_validation::parse_object_definition_text(raw, cid)
    }

    async fn load_object_definition_from_cid(&self, cid: &str) -> Result<ObjectDefinition> {
        let kubo_url = self.kubo_url().await;
        let raw = kubo::cat_cid(&kubo_url, cid).await
            .map_err(|e| anyhow!("failed to load object definition {}: {}", cid, e))?;
        Self::parse_object_definition_text(&raw, cid)
    }

    async fn resolve_object_cid_or_yaml_input(
        &self,
        value: &str,
    ) -> Result<(String, ObjectDefinition, bool)> {
        let input = value.trim();
        if input.is_empty() {
            return Err(anyhow!("missing object definition payload"));
        }

        match self.load_object_definition_from_cid(input).await {
            Ok(definition) => Ok((input.to_string(), definition, false)),
            Err(cid_err) => {
                let decoded = B64.decode(input.as_bytes()).map_err(|b64_err| {
                    anyhow!(
                        "not a valid CID ({}) and not valid base64 YAML ({})",
                        cid_err,
                        b64_err
                    )
                })?;
                let yaml = String::from_utf8(decoded)
                    .map_err(|utf8_err| anyhow!("invalid UTF-8 YAML payload: {}", utf8_err))?;

                let definition = Self::parse_object_definition_text(&yaml, "inline-content")
                    .map_err(|err| anyhow!("invalid object definition content: {}", err))?;

                let kubo_url = self.kubo_url().await;
                let cid = ipfs_add(&kubo_url, yaml.into_bytes())
                    .await
                    .map_err(|err| anyhow!("failed to publish object definition: {}", err))?;

                Ok((cid, definition, true))
            }
        }
    }

    async fn resolve_room_cid_or_yaml_input(&self, value: &str) -> Result<(String, String, bool)> {
        let input = value.trim();
        if input.is_empty() {
            return Err(anyhow!("missing room payload"));
        }

        let kubo_url = self.kubo_url().await;
        match kubo::cat_cid(&kubo_url, input).await {
            Ok(yaml_text) => Ok((input.to_string(), yaml_text, false)),
            Err(cid_err) => {
                let decoded = B64.decode(input.as_bytes()).map_err(|b64_err| {
                    anyhow!(
                        "not a valid CID ({}) and not valid base64 YAML ({})",
                        cid_err,
                        b64_err
                    )
                })?;
                let yaml_text = String::from_utf8(decoded)
                    .map_err(|utf8_err| anyhow!("invalid UTF-8 room YAML payload: {}", utf8_err))?;

                let published_cid = ipfs_add(&kubo_url, yaml_text.as_bytes().to_vec())
                    .await
                    .map_err(|err| anyhow!("failed to publish room YAML: {}", err))?;

                Ok((published_cid, yaml_text, true))
            }
        }
    }

    async fn hydrate_object_definition_by_cid(
        &self,
        room_name: &str,
        object_id: &str,
    ) -> Result<()> {
        let cid_to_load = {
            let objects = self.room_objects.read().await;
            let Some(room_map) = objects.get(room_name) else {
                return Ok(());
            };
            let Some(object) = room_map.get(object_id) else {
                return Ok(());
            };
            if object.definition.is_some() {
                return Ok(());
            }
            object.cid.clone()
        };

        let Some(cid) = cid_to_load else {
            return Ok(());
        };

        let definition = self.load_object_definition_from_cid(&cid).await?;

        let mut objects = self.room_objects.write().await;
        if let Some(room_map) = objects.get_mut(room_name) {
            if let Some(object) = room_map.get_mut(object_id) {
                if object.definition.is_none()
                    && object.cid.as_deref() == Some(cid.as_str())
                {
                    object.definition = Some(definition);
                }
            }
        }

        Ok(())
    }

    pub async fn enqueue_knock(
        &self,
        room: &str,
        requester_did: &str,
        requester_endpoint: &str,
        preferred_handle: Option<String>,
    ) -> (u64, bool) {
        let mut inbox = self.knock_inbox.write().await;
        if let Some(existing) = inbox
            .iter()
            .find(|item| {
                item.status == KnockStatus::Pending
                    && item.requester_did == requester_did
                    && item.room == room
            })
            .cloned()
        {
            return (existing.id, true);
        }

        let mut next = self.next_knock_id.write().await;
        *next += 1;
        let id = *next;
        drop(next);

        if inbox.len() >= MAX_KNOCK_INBOX {
            inbox.pop_front();
        }

        let knock = KnockMessage {
            id,
            room: room.to_string(),
            requester_did: requester_did.to_string(),
            requester_endpoint: requester_endpoint.to_string(),
            preferred_handle,
            requested_at: Utc::now().to_rfc3339(),
            status: KnockStatus::Pending,
            decision_note: None,
            decided_at: None,
        };

        inbox.push_back(knock.clone());
        drop(inbox);

        let mailbox_message = ObjectInboxMessage {
            id: knock.id,
            from_did: Some(knock.requester_did.clone()),
            from_object: None,
            kind: ObjectMessageKind::Command,
            body: format!("knock from {} for room {}", knock.requester_did, knock.room),
            sent_at: knock.requested_at.clone(),
            content_type: Some("application/x-ma-knock".to_string()),
            session_id: None,
            reply_to_request_id: None,
            retention: ObjectMessageRetention::Durable,
        };
        if self.find_intrinsic_mailbox_location().await.is_none() {
            self.ensure_lobby_intrinsic_objects().await;
        }

        if let Some((mailbox_room, mailbox_object_id)) = self.find_intrinsic_mailbox_location().await {
            let _ = self
                .enqueue_object_durable_inbox_message(&mailbox_room, &mailbox_object_id, mailbox_message)
                .await;
        }

        (id, false)
    }

    async fn list_knocks(&self, pending_only: bool) -> Vec<KnockMessage> {
        let inbox = self.knock_inbox.read().await;
        inbox
            .iter()
            .filter(|item| !pending_only || item.status == KnockStatus::Pending)
            .cloned()
            .collect()
    }

    async fn accept_knock(&self, id: u64) -> Result<KnockMessage> {
        let (accepted, requester_did) = {
            let mut inbox = self.knock_inbox.write().await;
            let Some(item) = inbox.iter_mut().find(|item| item.id == id) else {
                return Err(anyhow!("knock id {} not found", id));
            };
            if item.status != KnockStatus::Pending {
                return Err(anyhow!("knock id {} is not pending", id));
            }
            item.status = KnockStatus::Accepted;
            item.decided_at = Some(Utc::now().to_rfc3339());
            (item.clone(), item.requester_did.clone())
        };

        self.allow_entry_did(&requester_did).await;

        Ok(accepted)
    }

    async fn reject_knock(&self, id: u64, note: Option<String>) -> Result<KnockMessage> {
        let mut inbox = self.knock_inbox.write().await;
        let Some(item) = inbox.iter_mut().find(|item| item.id == id) else {
            return Err(anyhow!("knock id {} not found", id));
        };
        if item.status != KnockStatus::Pending {
            return Err(anyhow!("knock id {} is not pending", id));
        }

        item.status = KnockStatus::Rejected;
        item.decided_at = Some(Utc::now().to_rfc3339());
        item.decision_note = note;
        Ok(item.clone())
    }

    async fn delete_knock(&self, id: u64) -> Result<()> {
        let mut inbox = self.knock_inbox.write().await;
        let before = inbox.len();
        inbox.retain(|item| item.id != id);
        if inbox.len() == before {
            return Err(anyhow!("knock id {} not found", id));
        }
        Ok(())
    }

    /// Load all rooms from a world root index CID.
    /// New format stores DAG-CBOR links; legacy format stores YAML room_name → CID.
    /// Existing room avatars are preserved; IPFS data wins for everything else.
    pub async fn load_from_world_cid(&self, root_cid: &str) -> Result<usize> {
        let kubo_url = self.kubo_url().await;
        let (index_rooms, loaded_legacy_yaml, had_embedded_room_metadata): (HashMap<String, WorldRootRoomEntry>, bool, bool) =
            match dag_get_dag_cbor::<WorldRootIndexDag>(&kubo_url, root_cid).await {
                Ok(dag) => {
                    *self.state_cid.write().await = dag.state_cid.clone();
                    *self.lang_cid.write().await = dag.lang_cid.clone();
                    let mut had_embedded = false;
                    let rooms = dag
                        .rooms
                        .into_iter()
                        .map(|(name, value)| {
                            let entry = match value {
                                WorldRootRoomDagValue::Link(link) => WorldRootRoomEntry {
                                    cid: link.cid,
                                    ..Default::default()
                                },
                                WorldRootRoomDagValue::Entry(entry) => {
                                    had_embedded = true;
                                    entry
                                }
                            };
                            (name, entry)
                        })
                        .collect();
                    (rooms, false, had_embedded)
                }
                Err(_) => {
                    let yaml = kubo::cat_cid(&kubo_url, root_cid).await?;
                    let legacy: WorldRootIndex = serde_yaml::from_str(&yaml)
                        .map_err(|e| anyhow!("invalid world root index at {}: {}", root_cid, e))?;
                    let rooms = legacy
                        .rooms
                        .into_iter()
                        .map(|(name, cid)| {
                            (
                                name,
                                WorldRootRoomEntry {
                                    cid,
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                    (
                        rooms,
                        true,
                        false,
                    )
                }
            };
        *self.world_cid.write().await = Some(root_cid.to_string());

        let mut loaded = 0usize;
        let mut rooms_needing_rewrite: Vec<String> = Vec::new();
        for (room_name, room_entry) in &index_rooms {
            if !is_valid_nanoid_id(room_name) {
                warn!(
                    "Skipping room '{}' from world index {}: invalid nanoid id",
                    room_name, root_cid
                );
                continue;
            }
            let room_cid = &room_entry.cid;
            match kubo::cat_cid(&kubo_url, room_cid).await {
                Err(e) => warn!("Skipping room '{}' — failed to fetch {}: {}", room_name, room_cid, e),
                Ok(room_yaml) => match self.materialize_room_from_yaml(room_name, &room_yaml).await {
                    Err(e) => warn!("Skipping room '{}' — invalid YAML at {}: {}", room_name, room_cid, e),
                    Ok((mut loaded_room, needs_rewrite)) => {
                        if let Some(did) = room_entry.did.as_deref() {
                            let trimmed = did.trim();
                            if trimmed.is_empty() {
                                // Keep parsed room DID from room content if entry metadata is empty.
                            } else if Did::try_from(trimmed).is_ok() {
                                loaded_room.did = trimmed.to_string();
                            } else {
                                warn!(
                                    "Ignoring invalid room DID metadata for '{}' in world index {}: {}",
                                    room_name,
                                    root_cid,
                                    trimmed
                                );
                            }
                        }
                        if room_entry.owner_cid.trim().is_empty() {
                            return Err(anyhow!(
                                "missing owner_cid metadata for room '{}' in world index {}",
                                room_name,
                                root_cid
                            ));
                        }
                        if room_entry.acl_cid.trim().is_empty() {
                            return Err(anyhow!(
                                "missing acl_cid metadata for room '{}' in world index {}",
                                room_name,
                                root_cid
                            ));
                        }

                        let owner_yaml = kubo::cat_cid(&kubo_url, &room_entry.owner_cid).await
                            .map_err(|e| anyhow!(
                                "failed loading owner metadata {} for room '{}': {}",
                                room_entry.owner_cid,
                                room_name,
                                e
                            ))?;
                        let owner_doc: RoomOwnerDoc = serde_yaml::from_str(&owner_yaml)
                            .map_err(|e| anyhow!(
                                "invalid owner doc at {} for room '{}': {}",
                                room_entry.owner_cid,
                                room_name,
                                e
                            ))?;
                        if owner_doc.kind != "ma_room_owner" || owner_doc.version != 1 {
                            return Err(anyhow!(
                                "unsupported owner doc kind/version at {} for room '{}'",
                                room_entry.owner_cid,
                                room_name
                            ));
                        }

                        let acl_yaml = kubo::cat_cid(&kubo_url, &room_entry.acl_cid).await
                            .map_err(|e| anyhow!(
                                "failed loading acl {} for room '{}': {}",
                                room_entry.acl_cid,
                                room_name,
                                e
                            ))?;
                        let acl_doc: RoomAclDoc = serde_yaml::from_str(&acl_yaml)
                            .map_err(|e| anyhow!(
                                "invalid ACL doc at {} for room '{}': {}",
                                room_entry.acl_cid,
                                room_name,
                                e
                            ))?;
                        if acl_doc.kind != "ma_room_acl" || acl_doc.version != 1 {
                            return Err(anyhow!(
                                "unsupported ACL doc kind/version at {} for room '{}'",
                                room_entry.acl_cid,
                                room_name
                            ));
                        }

                        let mut loaded_acl = acl_doc.acl;
                        loaded_acl.owner = owner_doc.owner_did;
                        loaded_acl.owner_assertion_key = owner_doc.owner_assertion_key;
                        if let Some(owner) = loaded_acl.owner.clone() {
                            loaded_acl.allow.insert(owner.clone());
                            loaded_acl.deny.remove(&owner);
                        }
                        loaded_room.acl = loaded_acl;
                        {
                            let mut rooms = self.rooms.write().await;
                            if let Some(existing) = rooms.get(room_name) {
                                loaded_room.avatars = existing.avatars.clone();
                            }
                            rooms.insert(room_name.clone(), loaded_room);
                        }
                        {
                            let mut room_evs = self.room_events.write().await;
                            room_evs.entry(room_name.clone())
                                .or_insert_with(|| VecDeque::with_capacity(MAX_EVENTS));
                        }
                        self.room_cids.write().await.insert(room_name.clone(), room_cid.clone());
                        if needs_rewrite {
                            rooms_needing_rewrite.push(room_name.clone());
                        }
                        loaded += 1;
                        info!("Loaded room '{}' from CID {}", room_name, room_cid);
                    }
                },
            }
        }

        if !rooms_needing_rewrite.is_empty() {
            rooms_needing_rewrite.sort();
            rooms_needing_rewrite.dedup();
            match self.save_rooms_and_world_index(&rooms_needing_rewrite).await {
                Ok(new_cid) => {
                    info!(
                        "Migrated room snapshots for {:?} and updated world root index {} -> {}",
                        rooms_needing_rewrite,
                        root_cid,
                        new_cid
                    );
                }
                Err(err) => {
                    warn!(
                        "Loaded world root index {}, but room snapshot migration failed: {}",
                        root_cid,
                        err
                    );
                }
            }
        } else if loaded_legacy_yaml || had_embedded_room_metadata {
            match self.save_world_index().await {
                Ok(new_cid) => {
                    info!(
                        "Migrated world root index {} -> compact link map {}",
                        root_cid, new_cid
                    );
                }
                Err(err) => {
                    warn!(
                        "Loaded world root index {}, but compact re-write failed: {}",
                        root_cid,
                        err
                    );
                }
            }
        }

        Ok(loaded)
    }

    /// Serialize the current room_cids map as a root index, put it in IPFS,
    /// and write the resulting CID back to the on-disk config file.
    pub async fn save_world_index(&self) -> Result<String> {
        let kubo_url = self.kubo_url().await;
        let previous_world_cid = self.world_cid.read().await.clone();
        let pin_name = self.world_root_pin_name.read().await.clone();
        let room_cids = self.room_cids.read().await.clone();
        let room_meta: HashMap<String, (String, Option<String>, Option<String>, RoomAcl)> = self
            .rooms
            .read()
            .await
            .iter()
            .map(|(name, room)| {
                (
                    name.clone(),
                    (
                        room.did.clone(),
                        room.acl.owner.clone(),
                        room.acl.owner_assertion_key.clone(),
                        room.acl.clone(),
                    ),
                )
            })
            .collect();
        let mut rooms_index: HashMap<String, WorldRootRoomDagValue> = HashMap::new();
        for (name, cid) in room_cids {
            if !is_valid_nanoid_id(&name) {
                warn!("Skipping invalid room id '{}' while saving world index", name);
                continue;
            }

            let (did, owner_did, owner_assertion_key, mut acl) = room_meta
                .get(&name)
                .cloned()
                .unwrap_or_else(|| (String::new(), None, None, RoomAcl::open()));

            let owner_doc = RoomOwnerDoc {
                kind: "ma_room_owner".to_string(),
                version: 1,
                owner_did,
                owner_assertion_key,
            };
            let owner_yaml = serde_yaml::to_string(&owner_doc)
                .map_err(|e| anyhow!("failed to serialize owner metadata for room '{}': {}", name, e))?;
            let owner_cid = kubo::ipfs_add(&kubo_url, owner_yaml.into_bytes()).await?;

            // Owner metadata is persisted separately from ACL metadata.
            acl.owner = None;
            acl.owner_assertion_key = None;

            let acl_doc = RoomAclDoc {
                kind: "ma_room_acl".to_string(),
                version: 1,
                acl,
            };
            let acl_yaml = serde_yaml::to_string(&acl_doc)
                .map_err(|e| anyhow!("failed to serialize ACL for room '{}': {}", name, e))?;
            let acl_cid = kubo::ipfs_add(&kubo_url, acl_yaml.into_bytes()).await?;

            rooms_index.insert(
                name,
                WorldRootRoomDagValue::Entry(WorldRootRoomEntry {
                    cid,
                    title: None,
                    description: None,
                    did: if did.trim().is_empty() { None } else { Some(did) },
                    owner_cid,
                    acl_cid,
                }),
            );
        }

        let index = WorldRootIndexDag {
            rooms: rooms_index,
            state_cid: self.state_cid.read().await.clone(),
            lang_cid: self.lang_cid.read().await.clone(),
        };
        let new_cid = kubo::dag_put_dag_cbor(&kubo_url, &index).await?;

        // Keep exactly one named recursive pin for the world root index.
        if let Some(old_cid) = previous_world_cid.as_deref() {
            if old_cid != new_cid {
                if let Err(update_err) = pin_update(&kubo_url, old_cid, &new_cid).await {
                    warn!(
                        "pin/update failed for world root ({} -> {}): {}. Falling back to add+rm.",
                        old_cid,
                        new_cid,
                        update_err
                    );
                    pin_add_named(&kubo_url, &new_cid, &pin_name).await?;
                    if let Err(rm_err) = pin_rm(&kubo_url, old_cid).await {
                        warn!("pin/rm failed for previous world root {}: {}", old_cid, rm_err);
                    }
                }
            }
        } else {
            pin_add_named(&kubo_url, &new_cid, &pin_name).await?;
        }

        *self.world_cid.write().await = Some(new_cid.clone());
        info!("World root index updated: CID {}", new_cid);

        Ok(new_cid)
    }

    /// Persist a room's static snapshot (no runtime avatar state) and return CID.
    pub async fn save_room_static(&self, room_name: &str) -> Result<String> {
        let kubo_url = self.kubo_url().await;
        let room_yaml = {
            let rooms = self.rooms.read().await;
            let room = rooms
                .get(room_name)
                .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

            let mut exit_cids: HashMap<String, String> = HashMap::new();
            for exit in &room.exits {
                let exit_doc = ExitYamlDoc {
                    kind: "ma_exit".to_string(),
                    version: 1,
                    exit: exit.clone(),
                };
                let exit_yaml = serde_yaml::to_string(&exit_doc).map_err(|e| {
                    anyhow!(
                        "failed to serialize exit '{}' for room '{}': {}",
                        exit.id,
                        room_name,
                        e
                    )
                })?;
                let exit_cid = kubo::ipfs_add(&kubo_url, exit_yaml.into_bytes()).await?;
                exit_cids.insert(exit.id.clone(), exit_cid);
            }

            let room_doc = RoomYamlDocV2 {
                kind: "ma_room".to_string(),
                version: 2,
                id: room.name.clone(),
                titles: {
                    let mut titles = room.titles.clone();
                    if !titles.contains_key("und") {
                        titles.insert("und".to_string(), room.title_or_default());
                    }
                    titles
                },
                descriptions: {
                    let mut descriptions = room.descriptions.clone();
                    if !descriptions.contains_key("und") {
                        descriptions.insert("und".to_string(), room.description_or_default());
                    }
                    descriptions
                },
                did: None,
                exits: Vec::new(),
                exit_cids,
            };

            serde_yaml::to_string(&room_doc)
                .map_err(|e| anyhow!("failed to serialize room '{}' snapshot: {}", room_name, e))?
        };

        let room_cid = kubo::ipfs_add(&kubo_url, room_yaml.into_bytes()).await?;
        self.room_cids
            .write()
            .await
            .insert(room_name.to_string(), room_cid.clone());
        Ok(room_cid)
    }

    /// Persist changed room snapshots and then update world root index CID.
    pub async fn save_rooms_and_world_index(&self, room_names: &[String]) -> Result<String> {
        let mut seen = HashSet::new();
        for room_name in room_names {
            if seen.insert(room_name.clone()) {
                let cid = self.save_room_static(room_name).await?;
                info!("Room '{}' static snapshot pinned as {}", room_name, cid);
            }
        }
        self.save_world_index().await
    }

    pub async fn create_room(&self, name: String) -> Result<()> {
        if !is_valid_nanoid_id(&name) {
            return Err(anyhow!(
                "invalid room id '{}': room IDs must be nanoid-compatible ([A-Za-z0-9_-]+)",
                name
            ));
        }

        let did = self.build_room_did(&name).await;

        let mut rooms = self.rooms.write().await;
        if rooms.contains_key(&name) {
            return Err(anyhow!("Room {} already exists", name));
        }

        rooms.insert(name.clone(), Room::new(name.clone(), did));
        drop(rooms);

        let mut room_events = self.room_events.write().await;
        room_events.insert(name.clone(), VecDeque::with_capacity(MAX_EVENTS));
        drop(room_events);

        if name == DEFAULT_ROOM {
            self.ensure_lobby_intrinsic_objects().await;
        }

        self.record_event(format!("room created: {name}")).await;
        Ok(())
    }

    pub(crate) async fn join_room(
        &self,
        room_name: &str,
        req: AvatarRequest,
        preferred_handle: Option<String>,
    ) -> Result<String> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_name)
            .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

        let did_root = req.did.without_fragment().id();

        // Check room-level ACL.
        if !room.acl.can_enter(&did_root) {
            return Err(anyhow!("room ACL denied entry for {}", did_root));
        }

        // Same DID already present? Update endpoint and return existing handle.
        if let Some(existing) = room.avatars.get(&req.inbox) {
            if existing.agent_did == req.did {
                info!("[{}] @{} already present ({:?})", room_name, req.inbox, req.did);
                let handle = req.inbox.clone();
                return Ok(handle);
            }
        }

        let avatar = Avatar::new(
            req.inbox.clone(),
            req.did.clone(),
            req.agent_endpoint.clone(),
            req.language_order.clone(),
        );
        room.add_avatar(avatar);
        drop(rooms);

        // Resolve and register handle.
        let handle = self.register_handle(&did_root, preferred_handle, &req.inbox).await;

        info!("[{}] @{} joined ({:?}) from {}", room_name, handle, req.did, req.agent_endpoint);
        self.record_event(format!(
            "[{room_name}] @{} joined with {} from {}",
            handle,
            req.did.id(),
            req.agent_endpoint
        ))
        .await;
        self.record_room_event(
            room_name,
            "system",
            Some(handle.clone()),
            Some(did_root.clone()),
            Some(req.agent_endpoint.clone()),
            format!("{} entered {}", handle, room_name),
        )
        .await;
        Ok(handle)
    }

    /// Assign or recover a world-unique handle for `did_root`.
    /// The preferred_handle (from the client) or inbox fragment is the starting candidate.
    /// On collision with a different DID, appends the last 4 characters of the DID root.
    async fn register_handle(
        &self,
        did_root: &str,
        preferred: Option<String>,
        fragment: &str,
    ) -> String {
        // Same DID already has a handle? Return it.
        {
            let d2h = self.did_to_handle.read().await;
            if let Some(h) = d2h.get(did_root) {
                return h.clone();
            }
        }

        let candidate = preferred
            .filter(|h| !h.trim().is_empty())
            .unwrap_or_else(|| fragment.to_string());
        let candidate = candidate.trim().to_string();

        let handle = {
            let h2d = self.handle_to_did.read().await;
            if let Some(owner) = h2d.get(&candidate) {
                if owner == did_root {
                    candidate
                } else {
                    // Collision: disambiguate with last 4 chars of the DID root.
                    let suffix = if did_root.len() >= 4 { &did_root[did_root.len() - 4..] } else { did_root };
                    format!("{}_{}", candidate, suffix)
                }
            } else {
                candidate
            }
        };

        let mut h2d = self.handle_to_did.write().await;
        let mut d2h = self.did_to_handle.write().await;
        h2d.insert(handle.clone(), did_root.to_string());
        d2h.insert(did_root.to_string(), handle.clone());
        handle
    }

    /// Look up the handle for a root DID; falls back to the DID root id itself.
    pub(crate) async fn resolve_handle(&self, did_root: &str) -> String {
        let d2h = self.did_to_handle.read().await;
        d2h.get(did_root).cloned().unwrap_or_else(|| did_root.to_string())
    }

    /// Broadcast a signed chat message to room event log.
    pub async fn send_chat(
        &self,
        room_name: &str,
        sender_handle: &str,
        sender_did_root: &str,
        message_cbor: Vec<u8>,
    ) -> Result<()> {
        let rooms = self.rooms.read().await;
        let room = rooms
            .get(room_name)
            .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

        // Sender must be present in room.
        if !room.avatars.contains_key(sender_handle) {
            return Err(anyhow!(
                "sender @{} is not in room {} — enter first",
                sender_handle,
                room_name
            ));
        }
        drop(rooms);

        let cbor_b64 = B64.encode(&message_cbor);
        info!("[{}] {}: <chat>", room_name, sender_handle);
        self.record_event(format!("[{room_name}] {sender_handle}: <chat>")).await;

        // Build the room event directly so message_cbor_b64 is populated correctly.
        let mut next_sequence = self.next_room_event_sequence.write().await;
        *next_sequence += 1;
        let sequence = *next_sequence;
        drop(next_sequence);

        let entry = RoomEvent {
            sequence,
            room: room_name.to_string(),
            kind: "chat".to_string(),
            sender: Some(sender_handle.to_string()),
            sender_did: Some(sender_did_root.to_string()),
            sender_endpoint: None,
            message: String::new(),
            message_cbor_b64: Some(cbor_b64),
            occurred_at: Utc::now().to_rfc3339(),
        };

        let mut room_events = self.room_events.write().await;
        let events = room_events
            .entry(room_name.to_string())
            .or_insert_with(|| VecDeque::with_capacity(MAX_EVENTS));
        if events.len() >= MAX_EVENTS {
            events.pop_front();
        }
        events.push_back(entry);
        Ok(())
    }

    pub async fn leave_room(&self, room_name: &str, actor_name: &str) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_name)
            .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

        room.remove_avatar(actor_name);
        drop(rooms);

        info!("[{}] {} left", room_name, actor_name);
        self.record_event(format!("[{room_name}] {actor_name} left")).await;
        self.record_room_event(
            room_name,
            "system",
            Some(actor_name.to_string()),
            None,
            None,
            format!("{} left {}", actor_name, room_name),
        )
        .await;
        Ok(())
    }

    pub async fn send_message(
        &self,
        room_name: &str,
        from: &str,
        from_did: &Did,
        sender_profile: &str,
        envelope: MessageEnvelope,
    ) -> Result<(String, bool, String)> {
        {
            let rooms = self.rooms.read().await;
            let room = rooms
                .get(room_name)
                .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

            let Some(avatar) = room.avatars.get(from) else {
                return Err(anyhow!("unknown avatar @{} in room {}", from, room_name));
            };
            if avatar.agent_did != from_did.without_fragment() {
                return Err(anyhow!(
                    "sender DID mismatch for @{} in room {}",
                    from,
                    room_name
                ));
            }
        }

        let (response, broadcasted, effective_room) = match envelope {
            MessageEnvelope::Chatter { text } => {
                let speech = normalize_spoken_text(&text);
                info!("[{}] {}: {}", room_name, from, speech);
                self.record_event(format!("[{room_name}] {from}: {speech}")).await;
                let rendered = format!("{}: {}", from, speech);
                self.record_room_event(room_name, "speech", Some(from.to_string()), Some(from_did.id()), None, speech.clone())
                    .await;
                (rendered, true, room_name.to_string())
            }
            MessageEnvelope::RoomCommand { command } => {
                let caller_root_did = from_did.without_fragment().id();
                let response = self
                    .room_command(room_name, &command, from, sender_profile, Some(caller_root_did.as_str()))
                    .await;
                info!("[{}] {} -> @here: {} -> {}", room_name, from, command, response);
                self.record_event(format!("[{room_name}] {from} -> @here: {command} => {}", response))
                    .await;
                (response, false, room_name.to_string())
            }
            MessageEnvelope::ActorCommand { target, command } => {
                let broadcasted = matches!(command, ActorCommand::Say { .. });
                let speech_text = match &command {
                    ActorCommand::Say { payload } => Some(normalize_spoken_text(payload)),
                    ActorCommand::Raw { .. } => None,
                };
                let (response, effective_room) = self
                    .handle_actor_command(room_name, from, from_did, sender_profile, &target, command)
                    .await;
                info!("[{}] {} -> @{} -> {}", room_name, from, target, response);
                self.record_event(format!(
                    "[{room_name}] {from} -> @{target} => {}",
                    response.replace('\n', " ")
                ))
                .await;
                if broadcasted {
                    self.record_room_event(
                        room_name,
                        "speech",
                        Some(from.to_string()),
                        Some(from_did.id()),
                        None,
                        speech_text.unwrap_or_else(|| response.clone()),
                    )
                        .await;
                }
                (response, broadcasted, effective_room)
            }
        };

        Ok((response, broadcasted, effective_room))
    }

    async fn room_events_since(&self, room_name: &str, since_sequence: u64) -> Result<(Vec<RoomEvent>, u64)> {
        let room_events = self.room_events.read().await;
        let Some(events) = room_events.get(room_name) else {
            return Ok((Vec::new(), since_sequence));
        };

        let items = events
            .iter()
            .filter(|event| event.sequence > since_sequence)
            .cloned()
            .collect::<Vec<_>>();
        let latest = events.back().map(|event| event.sequence).unwrap_or(since_sequence);
        Ok((items, latest))
    }

    async fn latest_room_event_sequence(&self, room_name: &str) -> Result<u64> {
        let room_events = self.room_events.read().await;
        let latest = room_events
            .get(room_name)
            .and_then(|events| events.back().map(|e| e.sequence))
            .unwrap_or(0);
        Ok(latest)
    }

    async fn room_description(&self, room_name: &str) -> String {
        let rooms = self.rooms.read().await;
        rooms.get(room_name)
            .map(|r| r.description_or_default())
            .unwrap_or_default()
    }

    async fn room_title(&self, room_name: &str) -> String {
        let rooms = self.rooms.read().await;
        rooms.get(room_name)
            .map(|r| r.title_or_default())
            .unwrap_or_default()
    }

    async fn room_did(&self, room_name: &str) -> String {
        let rooms = self.rooms.read().await;
        rooms.get(room_name)
            .map(|r| r.did.clone())
            .unwrap_or_default()
    }

    async fn room_avatars(&self, room_name: &str) -> Vec<PresenceAvatar> {
        let rooms = self.rooms.read().await;
        let Some(room) = rooms.get(room_name) else { return Vec::new() };
        let mut avatars: Vec<PresenceAvatar> = room.avatars.iter()
            .map(|(handle, avatar)| PresenceAvatar {
                handle: handle.clone(),
                did: avatar.agent_did.id(),
            })
            .collect();
        avatars.sort_by(|a, b| a.handle.cmp(&b.handle));
        avatars
    }

    pub async fn owner_did(&self) -> Option<String> {
        self.owner_did.read().await.clone()
    }

    pub async fn set_owner_did(&self, did_raw: &str) -> Result<String> {
        let parsed = Did::try_from(did_raw.trim())
            .map_err(|e| anyhow!("invalid owner DID '{}': {}", did_raw, e))?;
        let root = parsed.without_fragment().id();
        *self.owner_did.write().await = Some(root.clone());
        self.allow_entry_did(&root).await;
        info!("World owner set to {}", root);
        Ok(root)
    }

    pub async fn world_cid(&self) -> Option<String> {
        self.world_cid.read().await.clone()
    }

    pub async fn state_cid(&self) -> Option<String> {
        self.state_cid.read().await.clone()
    }

    pub async fn lang_cid(&self) -> Option<String> {
        self.lang_cid.read().await.clone()
    }

    pub async fn set_lang_cid(&self, cid: Option<String>) {
        *self.lang_cid.write().await = cid.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    }

    pub async fn persisted_room_count(&self) -> usize {
        self.room_cids.read().await.len()
    }

    pub async fn did_ma_pointer_info(&self) -> (Option<String>, Option<String>, Option<String>) {
        let Some(world_did) = self.world_did.read().await.clone() else {
            return (None, None, Some("world DID is not configured".to_string()));
        };
        let kubo_url = self.kubo_url().await;
        match resolve_world_pointer_from_did(&kubo_url, &world_did).await {
            Ok((link, resolved)) => (link, resolved, None),
            Err(err) => (None, None, Some(err.to_string())),
        }
    }

    pub async fn last_pointer_publish_status(&self) -> (Option<bool>, Option<String>, Option<String>) {
        (
            self.last_pointer_publish_ok.read().await.clone(),
            self.last_pointer_publish_root_cid.read().await.clone(),
            self.last_pointer_publish_error.read().await.clone(),
        )
    }

    pub async fn ma_runtime_mode(&self) -> String {
        if ma_pointer_mode_enabled() {
            "pointer".to_string()
        } else {
            "inline".to_string()
        }
    }

    pub async fn save_encrypted_state(&self) -> Result<(String, String)> {
        let flushed = self.flush_dirty_object_blobs().await?;
        if flushed > 0 {
            info!("flushed {} dirty object blobs before save", flushed);
        }

        let kubo_url = self.kubo_url().await;
        let secrets = self.read_world_runtime_secrets().await?;
        let world_root = self
            .local_world_did_root()
            .await
            .ok_or_else(|| anyhow!("world DID root is not configured"))?;
        let signer_did = Did::try_from(world_root.as_str())
            .map_err(|e| anyhow!("invalid configured world DID root '{}': {}", world_root, e))?
            .without_fragment()
            .with_fragment("sig")
            .map_err(|e| anyhow!("failed building state signer DID: {}", e))?;
        let signing_key = SigningKey::from_private_key_bytes(
            signer_did.clone(),
            secrets.world_signing_private_key,
        )
            .map_err(|e| anyhow!("failed restoring state signing key: {}", e))?;

        let rooms_snapshot = {
            let rooms = self.rooms.read().await;
            let mut out = HashMap::new();
            for (room_id, room) in rooms.iter() {
                let avatars = room
                    .avatars
                    .values()
                    .map(|avatar| AvatarStateDoc {
                        inbox: avatar.inbox.clone(),
                        agent_did: avatar.agent_did.id(),
                        agent_endpoint: avatar.agent_endpoint.clone(),
                        language_order: avatar.language_order.clone(),
                        owner: avatar.owner.clone(),
                        descriptions: avatar.descriptions.clone(),
                        acl: avatar.acl.clone(),
                    })
                    .collect::<Vec<_>>();

                out.insert(
                    room_id.clone(),
                    RoomStateDoc {
                        name: room.name.clone(),
                        titles: room.titles.clone(),
                        exits: room.exits.clone(),
                        descriptions: room.descriptions.clone(),
                        did: room.did.clone(),
                        avatars,
                    },
                );
            }
            out
        };

        let events = self.events.read().await.iter().cloned().collect::<Vec<_>>();
        let room_events = self
            .room_events
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.iter().cloned().collect::<Vec<_>>()))
            .collect::<HashMap<_, _>>();
        let next_room_event_sequence = *self.next_room_event_sequence.read().await;
        let handle_to_did = self.handle_to_did.read().await.clone();
        let did_to_handle = self.did_to_handle.read().await.clone();
        let owner_did = self.owner_did.read().await.clone();
        let room_cids = self.room_cids.read().await.clone();
        let room_objects = self
            .room_objects
            .read()
            .await
            .iter()
            .map(|(room, objects)| {
                (
                    room.clone(),
                    objects
                        .values()
                        .map(|object| object.persisted_snapshot())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();

        let state = RuntimeStateDoc {
            kind: "ma_world_runtime_state".to_string(),
            version: 1,
            rooms: rooms_snapshot,
            events,
            room_events,
            next_room_event_sequence,
            handle_to_did,
            did_to_handle,
            owner_did,
            room_cids,
            room_objects,
            closet_profiles: self.closet_profiles.read().await.clone(),
            lang_cid: self.lang_cid.read().await.clone(),
        };

        let plaintext = serde_json::to_vec(&state)
            .map_err(|e| anyhow!("failed to serialize runtime state: {}", e))?;
        let signature = signing_key.sign(&plaintext);

        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new((&secrets.world_encryption_private_key).into());
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|_| anyhow!("failed to encrypt runtime state"))?;

        let envelope = PersistedWorldEnvelope {
            kind: "ma_world_state_envelope".to_string(),
            version: 1,
            created_at: Utc::now().to_rfc3339(),
            signer_did: signer_did.id(),
            signature_b64: B64.encode(signature),
            nonce_b64: B64.encode(nonce),
            ciphertext_b64: B64.encode(ciphertext),
        };

        let yaml = serde_yaml::to_string(&envelope)
            .map_err(|e| anyhow!("failed to serialize state envelope: {}", e))?;
        let state_cid = ipfs_add(&kubo_url, yaml.into_bytes()).await?;

        *self.state_cid.write().await = Some(state_cid.clone());

        let room_names = {
            let rooms = self.rooms.read().await;
            let mut names = rooms.keys().cloned().collect::<Vec<_>>();
            names.sort();
            names
        };

        let root_cid = if room_names.is_empty() {
            self.save_world_index().await?
        } else {
            self.save_rooms_and_world_index(&room_names).await?
        };

        let pointer_mode = ma_pointer_mode_enabled();
        if let Err(err) = publish_world_did_runtime_ma(
            &kubo_url,
            &self.world_root_pin_name().await,
            secrets.world_master_key,
            &state_cid,
            &root_cid,
            pointer_mode,
        )
        .await
        {
            let message = err.to_string();
            *self.last_pointer_publish_ok.write().await = Some(false);
            *self.last_pointer_publish_root_cid.write().await = Some(root_cid.clone());
            *self.last_pointer_publish_error.write().await = Some(message.clone());
            return Err(anyhow!(message));
        }

        *self.last_pointer_publish_ok.write().await = Some(true);
        *self.last_pointer_publish_root_cid.write().await = Some(root_cid.clone());
        *self.last_pointer_publish_error.write().await = None;

        Ok((state_cid, root_cid))
    }

    async fn flush_dirty_object_blobs(&self) -> Result<usize> {
        #[derive(Serialize)]
        struct BlobEnvelope<'a> {
            kind: &'static str,
            version: u32,
            #[serde(rename = "type")]
            blob_type: &'static str,
            content: &'a serde_json::Value,
        }

        let candidates = {
            let objects = self.room_objects.read().await;
            let mut out = Vec::new();
            for (room_id, room_map) in objects.iter() {
                for (object_id, object) in room_map.iter() {
                    if !(object.state_dirty || object.meta_dirty) {
                        continue;
                    }
                    out.push((
                        room_id.clone(),
                        object_id.clone(),
                        object.state_dirty,
                        object.meta_dirty,
                        object.state.clone(),
                        object.meta.clone(),
                    ));
                }
            }
            out
        };

        if candidates.is_empty() {
            return Ok(0);
        }

        let kubo_url = self.kubo_url().await;
        let mut updates: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();

        for (room_id, object_id, state_dirty, meta_dirty, state_value, meta_value) in candidates {
            let mut state_cid: Option<String> = None;
            let mut meta_cid: Option<String> = None;

            if state_dirty {
                let env = BlobEnvelope {
                    kind: "/ma/realms/1",
                    version: 1,
                    blob_type: "state",
                    content: &state_value,
                };
                let yaml = serde_yaml::to_string(&env)
                    .map_err(|e| anyhow!("failed to serialize object state blob: {}", e))?;
                match ipfs_add(&kubo_url, yaml.into_bytes()).await {
                    Ok(cid) => state_cid = Some(cid),
                    Err(err) => {
                        warn!(
                            "failed publishing state blob for object '{}' in room '{}': {}",
                            object_id,
                            room_id,
                            err
                        );
                    }
                }
            }

            if meta_dirty {
                let env = BlobEnvelope {
                    kind: "/ma/realms/1",
                    version: 1,
                    blob_type: "meta",
                    content: &meta_value,
                };
                let yaml = serde_yaml::to_string(&env)
                    .map_err(|e| anyhow!("failed to serialize object meta blob: {}", e))?;
                match ipfs_add(&kubo_url, yaml.into_bytes()).await {
                    Ok(cid) => meta_cid = Some(cid),
                    Err(err) => {
                        warn!(
                            "failed publishing meta blob for object '{}' in room '{}': {}",
                            object_id,
                            room_id,
                            err
                        );
                    }
                }
            }

            if state_cid.is_some() || meta_cid.is_some() {
                updates.push((room_id, object_id, state_cid, meta_cid));
            }
        }

        if updates.is_empty() {
            return Ok(0);
        }

        let mut applied = 0usize;
        let mut objects = self.room_objects.write().await;
        for (room_id, object_id, state_cid, meta_cid) in updates {
            let Some(room_map) = objects.get_mut(&room_id) else {
                continue;
            };
            let Some(object) = room_map.get_mut(&object_id) else {
                continue;
            };

            if let Some(cid) = state_cid {
                object.state_cid = Some(cid);
                object.state_dirty = false;
                applied = applied.saturating_add(1);
            }
            if let Some(cid) = meta_cid {
                object.meta_cid = Some(cid);
                object.meta_dirty = false;
                applied = applied.saturating_add(1);
            }
        }

        Ok(applied)
    }

    async fn flush_object_blobs(
        &self,
        room_name: &str,
        object_id: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        #[derive(Serialize)]
        struct BlobEnvelope<'a> {
            kind: &'static str,
            version: u32,
            #[serde(rename = "type")]
            blob_type: &'static str,
            content: &'a serde_json::Value,
        }

        let (state_dirty, meta_dirty, state_value, meta_value) = {
            let objects = self.room_objects.read().await;
            let room_map = objects
                .get(room_name)
                .ok_or_else(|| anyhow!("room '{}' not found", room_name))?;
            let object = room_map
                .get(object_id)
                .ok_or_else(|| anyhow!("object '{}' not found in room '{}'", object_id, room_name))?;
            (
                object.state_dirty,
                object.meta_dirty,
                object.state.clone(),
                object.meta.clone(),
            )
        };

        if !state_dirty && !meta_dirty {
            return Ok((None, None));
        }

        let kubo_url = self.kubo_url().await;
        let mut new_state_cid: Option<String> = None;
        let mut new_meta_cid: Option<String> = None;

        if state_dirty {
            let env = BlobEnvelope {
                kind: "/ma/realms/1",
                version: 1,
                blob_type: "state",
                content: &state_value,
            };
            let yaml = serde_yaml::to_string(&env)
                .map_err(|e| anyhow!("failed to serialize object state blob: {}", e))?;
            new_state_cid = Some(ipfs_add(&kubo_url, yaml.into_bytes()).await?);
        }

        if meta_dirty {
            let env = BlobEnvelope {
                kind: "/ma/realms/1",
                version: 1,
                blob_type: "meta",
                content: &meta_value,
            };
            let yaml = serde_yaml::to_string(&env)
                .map_err(|e| anyhow!("failed to serialize object meta blob: {}", e))?;
            new_meta_cid = Some(ipfs_add(&kubo_url, yaml.into_bytes()).await?);
        }

        let mut objects = self.room_objects.write().await;
        let room_map = objects
            .get_mut(room_name)
            .ok_or_else(|| anyhow!("room '{}' disappeared during flush", room_name))?;
        let object = room_map
            .get_mut(object_id)
            .ok_or_else(|| anyhow!("object '{}' disappeared during flush", object_id))?;

        if let Some(cid) = new_state_cid.clone() {
            object.state_cid = Some(cid);
            object.state_dirty = false;
        }
        if let Some(cid) = new_meta_cid.clone() {
            object.meta_cid = Some(cid);
            object.meta_dirty = false;
        }

        Ok((new_state_cid, new_meta_cid))
    }

    pub async fn load_encrypted_state(&self, state_cid: &str) -> Result<String> {
        let kubo_url = self.kubo_url().await;
        let secrets = self.read_world_runtime_secrets().await?;
        let world_root = self
            .local_world_did_root()
            .await
            .ok_or_else(|| anyhow!("world DID root is not configured"))?;
        let signer_did = Did::try_from(world_root.as_str())
            .map_err(|e| anyhow!("invalid configured world DID root '{}': {}", world_root, e))?
            .without_fragment()
            .with_fragment("sig")
            .map_err(|e| anyhow!("failed building state signer DID: {}", e))?;
        let signing_key = SigningKey::from_private_key_bytes(signer_did, secrets.world_signing_private_key)
            .map_err(|e| anyhow!("failed restoring state signing key: {}", e))?;

        let yaml = kubo::cat_cid(&kubo_url, state_cid).await?;
        let envelope: PersistedWorldEnvelope = serde_yaml::from_str(&yaml)
            .map_err(|e| anyhow!("invalid state envelope YAML at {}: {}", state_cid, e))?;

        if envelope.kind != "ma_world_state_envelope" || envelope.version != 1 {
            return Err(anyhow!(
                "unsupported state envelope kind/version at {}",
                state_cid
            ));
        }

        let nonce_raw = B64
            .decode(envelope.nonce_b64.as_bytes())
            .map_err(|e| anyhow!("invalid nonce in state envelope: {}", e))?;
        let nonce: [u8; 24] = nonce_raw
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid nonce length in state envelope"))?;
        let ciphertext = B64
            .decode(envelope.ciphertext_b64.as_bytes())
            .map_err(|e| anyhow!("invalid ciphertext in state envelope: {}", e))?;
        let signature = B64
            .decode(envelope.signature_b64.as_bytes())
            .map_err(|e| anyhow!("invalid signature in state envelope: {}", e))?;

        let cipher = XChaCha20Poly1305::new((&secrets.world_encryption_private_key).into());
        let plaintext = cipher
            .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| anyhow!("failed to decrypt state envelope: wrong key or tampered ciphertext"))?;

        let expected_signature = signing_key.sign(&plaintext);
        if signature != expected_signature {
            return Err(anyhow!(
                "state signature verification failed for {}",
                state_cid
            ));
        }

        let state: RuntimeStateDoc = serde_json::from_slice(&plaintext)
            .map_err(|e| anyhow!("invalid decrypted runtime state JSON: {}", e))?;
        if state.kind != "ma_world_runtime_state" || state.version != 1 {
            return Err(anyhow!("unsupported runtime state kind/version"));
        }

        let existing_room_acl: HashMap<String, RoomAcl> = self
            .rooms
            .read()
            .await
            .iter()
            .map(|(room_id, room)| (room_id.clone(), room.acl.clone()))
            .collect();

        let mut next_rooms = HashMap::new();
        for (room_id, room_doc) in state.rooms {
            if !is_valid_nanoid_id(&room_id) {
                return Err(anyhow!("invalid room id '{}' in runtime state", room_id));
            }

            let mut room = Room::new(room_doc.name, room_doc.did);
            room.titles = room_doc.titles;
            room.exits = room_doc.exits;
            room.acl = existing_room_acl
                .get(&room_id)
                .cloned()
                .unwrap_or_else(RoomAcl::open);
            if let Some(owner) = room.acl.owner.clone() {
                room.acl.allow.insert(owner.clone());
                room.acl.deny.remove(&owner);
            }
            room.descriptions = room_doc.descriptions;

            for avatar_doc in room_doc.avatars {
                let avatar_did = Did::try_from(avatar_doc.agent_did.as_str())
                    .map_err(|e| anyhow!("invalid avatar DID '{}': {}", avatar_doc.agent_did, e))?;
                let mut avatar = Avatar::new(
                    avatar_doc.inbox.clone(),
                    avatar_did,
                    avatar_doc.agent_endpoint,
                    avatar_doc.language_order,
                );
                avatar.owner = avatar_doc.owner;
                avatar.descriptions = avatar_doc.descriptions;
                avatar.acl = avatar_doc.acl;
                room.avatars.insert(avatar_doc.inbox, avatar);
            }

            next_rooms.insert(room_id, room);
        }

        let next_events = VecDeque::from(state.events);
        let mut next_room_events = HashMap::new();
        for (room_id, entries) in state.room_events {
            next_room_events.insert(room_id, VecDeque::from(entries));
        }

        let mut next_room_objects: HashMap<String, HashMap<String, ObjectRuntimeState>> = HashMap::new();
        for (room_id, object_list) in state.room_objects {
            let mut entries = HashMap::new();
            for mut object in object_list {
                object.clear_expired_lock(Utc::now().timestamp().max(0) as u64);
                if object.definition.is_none() {
                    if let Some(cid) = object.cid.as_deref() {
                        match self.load_object_definition_from_cid(cid).await {
                            Ok(definition) => {
                                object.definition = Some(definition);
                            }
                            Err(err) => {
                                warn!(
                                    "failed to hydrate object definition from CID {} for object '{}' in room '{}': {}",
                                    cid,
                                    object.id,
                                    room_id,
                                    err
                                );
                            }
                        }
                    }
                }
                entries.insert(object.id.clone(), object);
            }
            next_room_objects.insert(room_id, entries);
        }

        *self.rooms.write().await = next_rooms;
        *self.events.write().await = next_events;
        *self.room_events.write().await = next_room_events;
        *self.room_objects.write().await = next_room_objects;
        *self.next_room_event_sequence.write().await = state.next_room_event_sequence;
        *self.handle_to_did.write().await = state.handle_to_did;
        *self.did_to_handle.write().await = state.did_to_handle;
        let loaded_owner_did = state.owner_did;
        *self.owner_did.write().await = loaded_owner_did.clone();
        if let Some(owner) = loaded_owner_did {
            self.allow_entry_did(&owner).await;
        }
        *self.room_cids.write().await = state.room_cids;
        *self.closet_profiles.write().await = state.closet_profiles;
        *self.lang_cid.write().await = state.lang_cid;
        *self.state_cid.write().await = Some(state_cid.to_string());

        self.ensure_lobby_intrinsic_objects().await;

        let root_cid = self.save_world_index().await?;
        Ok(root_cid)
    }

    pub async fn snapshot(&self) -> WorldSnapshot {
        let rooms = self.rooms.read().await;
        let mut room_items = rooms
            .values()
            .map(|room| {
                let mut avatars = room
                    .avatars
                    .values()
                    .map(|avatar| AvatarSnapshot {
                        inbox: avatar.inbox.clone(),
                        agent_did: avatar.agent_did.id(),
                        agent_endpoint: avatar.agent_endpoint.clone(),
                        owner: avatar.owner.clone(),
                        description: avatar.description_or_default(),
                        acl: avatar.acl.summary(),
                        joined_at: format_system_time(avatar.joined_at),
                    })
                    .collect::<Vec<_>>();
                avatars.sort_by(|left, right| left.inbox.cmp(&right.inbox));

                RoomSnapshot {
                    name: room.name.clone(),
                    avatars,
                }
            })
            .collect::<Vec<_>>();
        room_items.sort_by(|left, right| left.name.cmp(&right.name));
        drop(rooms);

        let events = self.events.read().await.iter().cloned().collect();

        WorldSnapshot {
            rooms: room_items,
            recent_events: events,
        }
    }

    async fn handle_actor_command(
        &self,
        room_name: &str,
        from: &str,
        from_did: &Did,
        sender_profile: &str,
        target: &str,
        command: ActorCommand,
    ) -> (String, String) {
        match target {
            "here" | "room" => {
                let room_cmd = match command {
                    ActorCommand::Say { payload } => payload,
                    ActorCommand::Raw { command } => command,
                };
                let caller_root_did = from_did.without_fragment().id();
                (
                    self
                        .room_command(
                            room_name,
                            &room_cmd,
                            from,
                            sender_profile,
                            Some(caller_root_did.as_str()),
                        )
                        .await,
                    room_name.to_string(),
                )
            }
            "world" => {
                let cmd = match command {
                    ActorCommand::Say { payload } => payload,
                    ActorCommand::Raw { command } => command,
                };
                (
                    self
                        .handle_world_command(room_name, from, from_did, sender_profile, &cmd)
                        .await,
                    room_name.to_string(),
                )
            }
            "avatar" => self
                .handle_avatar_command(room_name, from, from_did, sender_profile, command)
                .await,
            _ => {
                let rooms = self.rooms.read().await;
                let Some(room) = rooms.get(room_name) else {
                    return (format!("@here room '{}' not found", room_name), room_name.to_string());
                };
                let shortcut_target = room
                    .avatars
                    .get(from)
                    .and_then(|avatar| avatar.resolve_object_shortcut(target));
                let mut actor_target = target.to_string();
                let mut actor_exists = room.avatars.contains_key(target) || target == from;
                if let Ok(did) = Did::try_from(target) {
                    let target_root = did.without_fragment().id();
                    if let Some((handle, _)) = room
                        .avatars
                        .iter()
                        .find(|(_, avatar)| avatar.agent_did.without_fragment().id() == target_root)
                    {
                        actor_target = handle.clone();
                        actor_exists = true;
                    }
                }
                drop(rooms);

                if let Some(resolved_target) = shortcut_target {
                    if let Some(result) = self
                        .handle_object_method(room_name, from, from_did, sender_profile, &resolved_target, command.clone())
                        .await
                    {
                        return result;
                    }
                    return (
                        format!("Object alias @{} is stale (object '{}' not found here).", target, resolved_target),
                        room_name.to_string(),
                    );
                }

                if !actor_exists {
                    if let Some(result) = self
                        .handle_object_method(room_name, from, from_did, sender_profile, target, command.clone())
                        .await
                    {
                        return result;
                    }
                    warn!("[{}] Unknown actor/object: @{}", room_name, target);
                    return (format!("Unknown actor or object: @{}", target), room_name.to_string());
                }

                match command {
                    ActorCommand::Say { payload } => {
                        let speech = normalize_spoken_text(&payload);
                        (format!("@{} says to @{}: {}", from, actor_target, speech), room_name.to_string())
                    }
                    ActorCommand::Raw { command } => {
                        (format!("@{} is here. Try '@{} say \"...\"'. (got: {})", actor_target, actor_target, command), room_name.to_string())
                    }
                }
            }
        }
    }

    async fn handle_object_method(
        &self,
        room_name: &str,
        from: &str,
        from_did: &Did,
        sender_profile: &str,
        target: &str,
        command: ActorCommand,
    ) -> Option<(String, String)> {
        let caller_root = from_did.without_fragment().id();
        let now_secs = Utc::now().timestamp().max(0) as u64;

        let object_id = {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let resolved_target = if let Ok(did) = Did::try_from(target.trim()) {
                let world_root = did.without_fragment().id();
                if !self.is_local_world_root(&world_root).await {
                    return None;
                }
                did.fragment.clone()?
            } else {
                let token = target.trim().trim_start_matches('@').to_ascii_lowercase();
                room_map
                    .values()
                    .find(|obj| obj.matches_target(token.as_str()))
                    .map(|obj| obj.id.clone())?
            };
            let object_id = resolved_target;
            if let Some(device) = room_map.get_mut(&object_id) {
                device.clear_expired_lock(now_secs);
            }
            object_id
        };

        if let Err(err) = self
            .hydrate_object_definition_by_cid(room_name, &object_id)
            .await
        {
            warn!(
                "failed to hydrate object definition for '{}' in room '{}': {}",
                object_id,
                room_name,
                err
            );
        }

        let object_label = {
            let objects = self.room_objects.read().await;
            let room_map = objects.get(room_name)?;
            let object = room_map.get(&object_id)?;
            format!("@{}", object.name)
        };

        let raw = match command {
            ActorCommand::Say { payload } => payload,
            ActorCommand::Raw { command } => command,
        };
        let trimmed = raw.trim();
        let parse_target = |token: &str| -> ObjectMessageTarget {
            let normalized = token.trim();
            if normalized.eq_ignore_ascii_case("room") {
                return ObjectMessageTarget::Room;
            }
            if normalized.eq_ignore_ascii_case("holder") {
                return ObjectMessageTarget::Holder;
            }
            if normalized.eq_ignore_ascii_case("caller") {
                return ObjectMessageTarget::Caller;
            }
            if let Some(object_id) = normalized.strip_prefix("object:") {
                return ObjectMessageTarget::Object(object_id.trim().to_string());
            }
            ObjectMessageTarget::Did(normalized.to_string())
        };

        let mut parts = trimmed.split_whitespace();
        let verb = parts.next().unwrap_or("help").to_ascii_lowercase();

        if object_id == "closet" {
            if matches!(verb.as_str(), "take" | "pickup" | "hold" | "drop" | "open" | "close") {
                return Some((
                    "@closet is fixed in the lobby and cannot be moved.".to_string(),
                    room_name.to_string(),
                ));
            }

            if matches!(verb.as_str(), "help" | "hjelp") {
                return Some((
                    format!("{} commands: {}", object_label, CLOSET_COMMANDS_INLINE),
                    room_name.to_string(),
                ));
            }

            if matches!(verb.as_str(), "show" | "status" | "look") {
                let listeners = self.closet_sessions.read().await.len();
                return Some((
                    format!(
                        "{} is fixed in the lobby. active_closet_sessions={} notice='{}'",
                        object_label, listeners, CLOSET_LISTENING_MESSAGE
                    ),
                    room_name.to_string(),
                ));
            }

            return Some((
                format!("{} commands: {}", object_label, CLOSET_COMMANDS_INLINE),
                room_name.to_string(),
            ));
        }

        let verb_requirements = {
            let objects = self.room_objects.read().await;
            let room_map = objects.get(room_name)?;
            let object = room_map.get(&object_id)?;
            Self::lookup_object_verb_definition(object, &verb)
                .map(|entry| entry.requirements)
                .unwrap_or_default()
        };

        let cap_verb = match verb.as_str() {
            "pickup" | "hold" => "take",
            "status" | "look" => "show",
            other => other,
        };
        let required_capability = if matches!(cap_verb, "help" | "show") {
            format!("object.{}.read", object_id)
        } else {
            format!("object.{}.method.{}.invoke", object_id, cap_verb)
        };

        match self
            .object_capability_allowed(room_name, &object_id, &caller_root, &required_capability)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                return Some((
                    format!("access denied for capability '{}'", required_capability),
                    room_name.to_string(),
                ));
            }
            Err(err) => {
                warn!(
                    "object ACL evaluation failed for '{}' in room '{}': {}",
                    object_id,
                    room_name,
                    err
                );
                return Some((
                    "access denied (invalid ACL policy)".to_string(),
                    room_name.to_string(),
                ));
            }
        }

        if !verb_requirements.is_empty() {
            let req_set = match RequirementSet::parse_many(&verb_requirements) {
                Ok(set) => set,
                Err(err) => {
                    return Some((
                        format!("invalid object requirements: {}", err),
                        room_name.to_string(),
                    ));
                }
            };

            let report = req_set.validate();
            if !report.is_ok() {
                let first_issue = report
                    .issues
                    .first()
                    .map(|issue| issue.message.clone())
                    .unwrap_or_else(|| "unknown requirements validation error".to_string());
                return Some((
                    format!("invalid object requirements: {}", first_issue),
                    room_name.to_string(),
                ));
            }

            let req_context = {
                let world_owner = self.owner_did.read().await.clone();
                let handle_to_did = self.handle_to_did.read().await.clone();
                let room_location = self.build_room_did(room_name).await;
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                let object = room_map.get(&object_id)?;
                let location = object
                    .holder
                    .as_ref()
                    .and_then(|holder| handle_to_did.get(holder).cloned())
                    .unwrap_or(room_location);
                ObjectRequirementRuntime {
                    room_name: room_name.to_string(),
                    user: caller_root.clone(),
                    owner: object.owner_did.clone().or_else(|| world_owner.clone()),
                    location,
                    opened_by: object.opened_by.clone(),
                    world_owner,
                }
            };

            let eval = req_set.evaluate(&req_context);
            if !eval.passed {
                let failed = eval
                    .failed
                    .iter()
                    .map(|req| req.render())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some((
                    format!("requirements not satisfied: {}", failed),
                    room_name.to_string(),
                ));
            }

            // Keep mailbox lock alive while caller executes verbs that require an open mailbox session.
            if req_set
                .all_of
                .iter()
                .any(|req| req.references_symbol("opened_by"))
            {
                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                if let Some(device) = room_map.get_mut(&object_id) {
                    device.lock_expires_at = Some(now_secs + MAILBOX_LOCK_SECS);
                }
            }
        }

        let declarative = {
            let objects = self.room_objects.read().await;
            let room_map = objects.get(room_name)?;
            let object = room_map.get(&object_id)?;
            Self::lookup_object_print_verb(object, &verb, sender_profile)
        };
        if let Some(output) = declarative {
            return Some((output, room_name.to_string()));
        }

        if verb == "help" {
            return Some((
                format!("{} commands: {}", object_label, MAILBOX_COMMANDS_INLINE),
                room_name.to_string(),
            ));
        }

        if verb == "set" {
            let sub = parts.next().unwrap_or_default().to_ascii_lowercase();
            let rest = parts.collect::<Vec<_>>().join(" ");
            let value = rest.trim();

            let (object_name, object_owner, is_world_owner) = {
                let owner = self.owner_did.read().await.clone();
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                let object = room_map.get(&object_id)?;
                (
                    object.name.clone(),
                    object.owner_did.clone(),
                    owner
                        .as_deref()
                        .map(|did| did == caller_root.as_str())
                        .unwrap_or(false),
                )
            };

            let is_object_owner = object_owner
                .as_deref()
                .map(|did| did == caller_root.as_str())
                .unwrap_or(false);
            if !is_object_owner && !is_world_owner {
                return Some((
                    format!("only @{} owner or world owner can set definition", object_name),
                    room_name.to_string(),
                ));
            }

            if sub == "cid" {
                if value.is_empty() {
                    return Some((
                        format!("usage: {} set cid <cid|base64-yaml>", object_label),
                        room_name.to_string(),
                    ));
                }

                let (cid, definition, published_from_yaml) = match self.resolve_object_cid_or_yaml_input(value).await {
                    Ok(tuple) => tuple,
                    Err(err) => {
                        return Some((
                            format!("invalid object definition payload: {}", err),
                            room_name.to_string(),
                        ));
                    }
                };

                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                let object = room_map.get_mut(&object_id)?;
                object.cid = Some(cid.clone());
                object.definition = Some(definition);
                object.meta_dirty = true;

                if published_from_yaml {
                    return Some((
                        format!("@{} cid published and set to {}", object.name, cid),
                        room_name.to_string(),
                    ));
                }
                return Some((
                    format!("@{} cid set to {}", object.name, cid),
                    room_name.to_string(),
                ));
            }

            if sub == "content" || sub == "content-b64" {
                if value.is_empty() {
                    return Some((
                        format!("usage: {} set content-b64 <base64-yaml>", object_label),
                        room_name.to_string(),
                    ));
                }

                let decoded = match B64.decode(value.as_bytes()) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        return Some((
                            format!("invalid base64 content: {}", err),
                            room_name.to_string(),
                        ));
                    }
                };
                let yaml = match String::from_utf8(decoded) {
                    Ok(text) => text,
                    Err(err) => {
                        return Some((
                            format!("invalid UTF-8 YAML payload: {}", err),
                            room_name.to_string(),
                        ));
                    }
                };

                let definition = match Self::parse_object_definition_text(&yaml, "inline-content") {
                    Ok(def) => def,
                    Err(err) => {
                        return Some((
                            format!("invalid object definition content: {}", err),
                            room_name.to_string(),
                        ));
                    }
                };

                let kubo_url = self.kubo_url().await;
                let cid = match ipfs_add(&kubo_url, yaml.into_bytes()).await {
                    Ok(cid) => cid,
                    Err(err) => {
                        return Some((
                            format!("failed to publish object definition: {}", err),
                            room_name.to_string(),
                        ));
                    }
                };

                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                let object = room_map.get_mut(&object_id)?;
                object.cid = Some(cid.clone());
                object.definition = Some(definition);
                object.meta_dirty = true;

                return Some((
                    format!("@{} cid published and set to {}", object.name, cid),
                    room_name.to_string(),
                ));
            }

            return Some((
                format!("usage: {} set cid <cid|base64-yaml> | {} set content-b64 <base64-yaml>", object_label, object_label),
                room_name.to_string(),
            ));
        }

        if verb == "show" || verb == "status" || verb == "look" {
            let (device_name, device_kind, object_did, cid, holder, opened_by, durable, persistence, durable_inbox_messages, ephemeral_inbox_messages, outbound_messages) = {
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                let device = room_map.get(&object_id)?;
                let world_did_root = self
                    .local_world_did_root()
                    .await
                    .unwrap_or_else(|| "did:ma:unconfigured".to_string());
                (
                    device.name.clone(),
                    device.kind.clone(),
                    format!("{}#{}", world_did_root, device.id),
                    device.cid.clone().unwrap_or_else(|| "(builtin)".to_string()),
                    device.holder.clone().unwrap_or_else(|| "(none)".to_string()),
                    device
                        .opened_by
                        .clone()
                        .unwrap_or_else(|| "(closed)".to_string()),
                    device.durable,
                    format!("{:?}", device.persistence).to_ascii_lowercase(),
                    device.durable_inbox_len(),
                    device.ephemeral_inbox_len(),
                    device.pending_outbox.len(),
                )
            };
            let pending = self.list_knocks(true).await.len();
            return Some((
                format!(
                    "@{} did={} kind={} cid={} holder={} opened_by={} durable={} persistence={} durable_inbox_messages={} ephemeral_inbox_messages={} outbound_messages={} pending_messages={}",
                    device_name, object_did, device_kind, cid, holder, opened_by, durable, persistence, durable_inbox_messages, ephemeral_inbox_messages, outbound_messages, pending
                ),
                room_name.to_string(),
            ));
        }

        if verb == "take" || verb == "pickup" || verb == "hold" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if let Some(holder) = device.holder.as_deref() {
                if holder != from {
                    return Some((format!("@{} is currently held by {}", device.name, holder), room_name.to_string()));
                }
            }
            device.holder = Some(from.to_string());
            device.state_dirty = true;
            return Some((format!("You pick up @{}.", device.name), room_name.to_string()));
        }

        if verb == "drop" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if device.holder.as_deref() != Some(from) {
                return Some((format!("You are not holding @{}.", device.name), room_name.to_string()));
            }
            device.holder = None;
            if device.opened_by.as_deref() == Some(caller_root.as_str()) {
                device.opened_by = None;
                device.locked_by = None;
                device.lock_expires_at = None;
            }
            device.state_dirty = true;
            return Some((format!("You drop @{}.", device.name), room_name.to_string()));
        }

        if verb == "open" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if device.holder.as_deref() != Some(from) {
                return Some((format!("You must hold @{} before opening it.", device.name), room_name.to_string()));
            }
            if let Some(locked_by) = device.locked_by.as_deref() {
                if locked_by != caller_root {
                    return Some((format!("@{} is locked by {}.", device.name, locked_by), room_name.to_string()));
                }
            }
            device.opened_by = Some(caller_root.clone());
            device.locked_by = Some(caller_root.clone());
            device.lock_expires_at = Some(now_secs + MAILBOX_LOCK_SECS);
            device.state_dirty = true;
            return Some((format!("@{} opened for {}", device.name, caller_root), room_name.to_string()));
        }

        if verb == "close" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if device.opened_by.as_deref() != Some(caller_root.as_str()) {
                return Some((format!("@{} is not open for your DID.", device.name), room_name.to_string()));
            }
            device.opened_by = None;
            device.locked_by = None;
            device.lock_expires_at = None;
            device.state_dirty = true;
            return Some((format!("@{} closed.", device.name), room_name.to_string()));
        }

        if verb == "flush" {
            let object_name = {
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                let object = room_map.get(&object_id)?;
                object.name.clone()
            };

            return Some(match self.flush_object_blobs(room_name, &object_id).await {
                Ok((None, None)) => (
                    format!("@{} flush: no dirty meta/state", object_name),
                    room_name.to_string(),
                ),
                Ok((state_cid, meta_cid)) => (
                    format!(
                        "@{} flush: state_cid={} meta_cid={}",
                        object_name,
                        state_cid.unwrap_or_else(|| "(unchanged)".to_string()),
                        meta_cid.unwrap_or_else(|| "(unchanged)".to_string())
                    ),
                    room_name.to_string(),
                ),
                Err(err) => (
                    format!("@{} flush failed: {}", object_name, err),
                    room_name.to_string(),
                ),
            });
        }

        if verb == "list" {
            let items = self.list_knocks(true).await;
            if items.is_empty() {
                return Some((format!("{} has no pending knock requests", object_label), room_name.to_string()));
            }
            let mut lines = Vec::new();
            for item in items {
                lines.push(format!(
                    "id={} room={} did={} at={}",
                    item.id, item.room, item.requester_did, item.requested_at
                ));
            }
            return Some((format!("{} pending:\n{}", object_label, lines.join("\n")), room_name.to_string()));
        }

        if verb == "pop" {
            let popped = self.pop_object_inbox_message(room_name, &object_id).await;
            return Some(match popped {
                Some(message) => {
                    let from = message
                        .from_did
                        .clone()
                        .or(message.from_object.clone())
                        .unwrap_or_else(|| "(unknown)".to_string());
                    (
                        format!(
                            "{} pop id={} from={} kind={:?} retention={:?} body={} reply_to={}",
                            object_label,
                            message.id,
                            from,
                            message.kind,
                            message.retention,
                            message.body,
                            message
                                .reply_to_request_id
                                .clone()
                                .unwrap_or_else(|| "(none)".to_string())
                        ),
                        room_name.to_string(),
                    )
                }
                None => (format!("{} pop: empty inbox", object_label), room_name.to_string()),
            });
        }

        if verb == "ask" {
            let args = trimmed
                .strip_prefix("ask")
                .map(str::trim)
                .unwrap_or_default();
            let mut split = args.splitn(2, char::is_whitespace);
            let Some(target_token) = split.next() else {
                return Some((
                    format!("usage: {} ask <room|holder|caller|did|object:id> <text>", object_label),
                    room_name.to_string(),
                ));
            };
            let text = split.next().unwrap_or_default().trim();
            if text.is_empty() {
                return Some((
                    format!("usage: {} ask <room|holder|caller|did|object:id> <text>", object_label),
                    room_name.to_string(),
                ));
            }

            let target = parse_target(target_token);
            let request_id = {
                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                let device = room_map.get_mut(&object_id)?;
                device.begin_ephemeral_request(
                    ObjectMessageIntent {
                        target,
                        kind: ObjectMessageKind::Whisper,
                        body: text.to_string(),
                        content_type: Some("application/x-ma-object-ephemeral".to_string()),
                        encrypted: false,
                        reply_to_message_id: None,
                        request_id: None,
                        session_id: Some(caller_root.clone()),
                        timeout_secs: Some(60),
                        attempt: 1,
                    },
                    now_secs,
                    60,
                )
            };

            return Some((
                format!("{} ask queued request_id={} timeout=60s", object_label, request_id),
                room_name.to_string(),
            ));
        }

        if verb == "retry" {
            let Some(request_id) = parts.next() else {
                return Some((format!("usage: {} retry <request_id>", object_label), room_name.to_string()));
            };
            let retried_attempt = {
                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                let device = room_map.get_mut(&object_id)?;
                device.retry_ephemeral_request(request_id, now_secs)
            };
            return Some(match retried_attempt {
                Some(attempt) => (
                    format!("{} retry queued request_id={} attempt={}", object_label, request_id, attempt),
                    room_name.to_string(),
                ),
                None => (
                    format!("{} retry failed request_id={} (missing or expired)", object_label, request_id),
                    room_name.to_string(),
                ),
            });
        }

        if verb == "reply" {
            let args = trimmed
                .strip_prefix("reply")
                .map(str::trim)
                .unwrap_or_default();
            let mut split = args.splitn(2, char::is_whitespace);
            let Some(request_id) = split.next() else {
                return Some((format!("usage: {} reply <request_id> <text>", object_label), room_name.to_string()));
            };
            let text = split.next().unwrap_or_default().trim();
            if text.is_empty() {
                return Some((format!("usage: {} reply <request_id> <text>", object_label), room_name.to_string()));
            }

            let (resolved, message_id) = {
                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                let device = room_map.get_mut(&object_id)?;
                let message_id = device
                    .inbox
                    .iter()
                    .map(|msg| msg.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                let reply_message = ObjectInboxMessage {
                    id: message_id,
                    from_did: Some(caller_root.clone()),
                    from_object: None,
                    kind: ObjectMessageKind::Whisper,
                    body: text.to_string(),
                    sent_at: Utc::now().to_rfc3339(),
                    content_type: Some("application/x-ma-object-ephemeral-reply".to_string()),
                    session_id: Some(caller_root.clone()),
                    reply_to_request_id: Some(request_id.to_string()),
                    retention: ObjectMessageRetention::Ephemeral,
                };
                let resolved = device.resolve_ephemeral_reply(&reply_message);
                device.push_ephemeral_inbox_message(reply_message, MAX_OBJECT_INBOX);
                (resolved, message_id)
            };

            return Some((
                if resolved {
                    format!("{} reply accepted request_id={} message_id={}", object_label, request_id, message_id)
                } else {
                    format!("{} reply queued but no matching pending request_id={} message_id={}", object_label, request_id, message_id)
                },
                room_name.to_string(),
            ));
        }

        if verb == "pending" {
            let summary = {
                let mut objects = self.room_objects.write().await;
                let room_map = objects.get_mut(room_name)?;
                let device = room_map.get_mut(&object_id)?;
                let expired = device.reap_expired_ephemeral_requests(now_secs);
                let mut rows = device
                    .pending_ephemeral_requests
                    .values()
                    .map(|pending| {
                        format!(
                            "request_id={} attempt={} expires_at={} session={}",
                            pending.request_id,
                            pending.attempt,
                            pending.expires_at_unix(),
                            pending
                                .session_id
                                .clone()
                                .unwrap_or_else(|| "(none)".to_string())
                        )
                    })
                    .collect::<Vec<_>>();
                rows.sort();
                if rows.is_empty() {
                    if expired.is_empty() {
                        format!("{} pending: (none)", object_label)
                    } else {
                        format!("{} pending: (none), expired={}", object_label, expired.join(","))
                    }
                } else {
                    let prefix = if expired.is_empty() {
                        format!("{} pending:", object_label)
                    } else {
                        format!("{} pending (expired={}):", object_label, expired.join(","))
                    };
                    format!("{}\n{}", prefix, rows.join("\n"))
                }
            };
            return Some((summary, room_name.to_string()));
        }

        if verb == "accept" {
            let Some(id_raw) = parts.next() else {
                return Some((format!("usage: {} accept <id>", object_label), room_name.to_string()));
            };
            let id = match Self::parse_knock_id_arg(id_raw) {
                Ok(v) => v,
                Err(err) => return Some((err, room_name.to_string())),
            };
            return Some((
                match self.accept_knock(id).await {
                    Ok(item) => format!("accepted knock id={} did={}", item.id, item.requester_did),
                    Err(err) => format!("accept failed: {}", err),
                },
                room_name.to_string(),
            ));
        }

        if verb == "reject" {
            let Some(id_raw) = parts.next() else {
                return Some((format!("usage: {} reject <id> [note]", object_label), room_name.to_string()));
            };
            let id = match Self::parse_knock_id_arg(id_raw) {
                Ok(v) => v,
                Err(err) => return Some((err, room_name.to_string())),
            };
            let note = {
                let rest = parts.collect::<Vec<_>>().join(" ");
                if rest.trim().is_empty() { None } else { Some(rest) }
            };
            return Some((
                match self.reject_knock(id, note).await {
                    Ok(item) => format!("rejected knock id={} did={}", item.id, item.requester_did),
                    Err(err) => format!("reject failed: {}", err),
                },
                room_name.to_string(),
            ));
        }

        if verb == "invite" {
            let Some(target_did_raw) = parts.next() else {
                return Some((format!("usage: {} invite <did> [note]", object_label), room_name.to_string()));
            };
            let target_root = match Self::parse_invite_root_did_arg(target_did_raw) {
                Ok(root) => root,
                Err(err) => return Some((err, room_name.to_string())),
            };
            self.allow_entry_did(&target_root).await;
            return Some((
                format!("invited {} (allowlisted)", target_root),
                room_name.to_string(),
            ));
        }

        Some((
            format!("{} commands: {}", object_label, MAILBOX_COMMANDS_INLINE),
            room_name.to_string(),
        ))
    }

    async fn handle_avatar_command(
        &self,
        room_name: &str,
        from: &str,
        from_did: &Did,
        sender_profile: &str,
        command: ActorCommand,
    ) -> (String, String) {
        match command {
            ActorCommand::Say { payload } => {
                // ' / say: broadcast speech to the room, formatted identically to Chatter.
                let speech = normalize_spoken_text(&payload);
                (format!("{}: {}", from, speech), room_name.to_string())
            }
            ActorCommand::Raw { command } => {
                let trimmed = command.trim();

                if let Some(rest) = trimmed.strip_prefix("use ") {
                    let Some((target_raw, alias_raw)) = rest.split_once(" as ") else {
                        return (
                            "usage: use <object|did:ma:...#object> as @alias".to_string(),
                            room_name.to_string(),
                        );
                    };

                    let target_value = target_raw.trim();
                    let alias = alias_raw.trim();
                    if !alias.starts_with('@') {
                        return (
                            "usage: use <object|did:ma:...#object> as @alias".to_string(),
                            room_name.to_string(),
                        );
                    }

                    let (object_id, object_did_id) = if target_value.starts_with("did:ma:") {
                        let object_did = match Did::try_from(target_value) {
                            Ok(did) => did,
                            Err(err) => {
                                return (
                                    format!("invalid object DID '{}': {}", target_value, err),
                                    room_name.to_string(),
                                );
                            }
                        };

                        let root = object_did.without_fragment().id();
                        if !self.is_local_world_root(&root).await {
                            return (
                                format!("object DID '{}' is not in this world", object_did.id()),
                                room_name.to_string(),
                            );
                        }

                        let Some(fragment) = object_did.fragment.clone() else {
                            return (
                                format!("object DID '{}' is missing #fragment", object_did.id()),
                                room_name.to_string(),
                            );
                        };

                        (fragment, object_did.id())
                    } else {
                        let token = target_value.trim().trim_start_matches('@').to_ascii_lowercase();
                        let maybe_object_id = {
                            let objects = self.room_objects.read().await;
                            objects
                                .get(room_name)
                                .and_then(|room_map| {
                                    room_map
                                        .values()
                                        .find(|obj| obj.matches_target(token.as_str()))
                                        .map(|obj| obj.id.clone())
                                })
                        };
                        let Some(object_id) = maybe_object_id else {
                            return (
                                format!("object '{}' is not present in room '{}'.", target_value, room_name),
                                room_name.to_string(),
                            );
                        };
                        let world_root = self
                            .local_world_did_root()
                            .await
                            .unwrap_or_else(|| "did:ma:unconfigured".to_string());
                        (object_id.clone(), format!("{}#{}", world_root, object_id))
                    };

                    let object_exists_here = {
                        let objects = self.room_objects.read().await;
                        objects
                            .get(room_name)
                            .map(|room_map| room_map.contains_key(&object_id))
                            .unwrap_or(false)
                    };

                    if !object_exists_here {
                        return (
                            format!("object '{}' is not present in room '{}'.", object_id, room_name),
                            room_name.to_string(),
                        );
                    }

                    let shortcuts_summary = {
                        let mut rooms = self.rooms.write().await;
                        let Some(room) = rooms.get_mut(room_name) else {
                            return (format!("@here room '{}' not found", room_name), room_name.to_string());
                        };
                        let Some(avatar) = room.avatars.get_mut(from) else {
                            return (format!("@avatar '{}' not found", from), room_name.to_string());
                        };
                        if !avatar.bind_object_shortcut(alias, &object_id) {
                            return (
                                format!("invalid alias '{}'. Use @alias with [A-Za-z0-9_-].", alias),
                                room_name.to_string(),
                            );
                        }
                        avatar.object_shortcuts_summary()
                    };

                    return (
                        format!(
                            "bound {} -> {} (object_id={}) shortcuts=[{}]",
                            alias,
                            object_did_id,
                            object_id,
                            shortcuts_summary
                        ),
                        room_name.to_string(),
                    );
                }

                if let Some(alias_raw) = trimmed.strip_prefix("unuse ") {
                    let alias = alias_raw.trim();
                    if alias.is_empty() {
                        return (
                            "usage: unuse @alias".to_string(),
                            room_name.to_string(),
                        );
                    }

                    let (removed, shortcuts_summary) = {
                        let mut rooms = self.rooms.write().await;
                        let Some(room) = rooms.get_mut(room_name) else {
                            return (format!("@here room '{}' not found", room_name), room_name.to_string());
                        };
                        let Some(avatar) = room.avatars.get_mut(from) else {
                            return (format!("@avatar '{}' not found", from), room_name.to_string());
                        };
                        let removed = avatar.remove_object_shortcut(alias);
                        (removed, avatar.object_shortcuts_summary())
                    };

                    return if removed {
                        (
                            format!("removed shortcut {} shortcuts=[{}]", alias, shortcuts_summary),
                            room_name.to_string(),
                        )
                    } else {
                        (
                            format!("shortcut {} not found", alias),
                            room_name.to_string(),
                        )
                    };
                }

                // Movement: `go <direction>` or bare direction name.
                let direction = trimmed
                    .strip_prefix("go ")
                    .map(str::trim)
                    .unwrap_or(trimmed);

                let caller_root_did = from_did.without_fragment().id();

                let move_target = {
                    let rooms = self.rooms.read().await;
                    rooms.get(room_name).and_then(|room| {
                        room.exits
                            .iter()
                            .find(|e| e.matches_for_preferences(direction, &["und".to_string()]))
                            .cloned()
                    })
                };

                if let Some(exit) = move_target {
                    let exit_name = exit.name_for_preferences(&["und".to_string()]);
                    if exit.locked {
                        return (format!("The way {} is locked.", exit_name), room_name.to_string());
                    }

                    if !exit.can_use(&caller_root_did) {
                        return (format!("You are not allowed to use exit '{}'.", exit_name), room_name.to_string());
                    }

                    let destination = exit.to.clone();
                    let travel_text = exit.travel_text_for_preferences(&["und".to_string()]);

                    // Exit destinations may be local room fragments or full room DIDs.
                    // If the DID root is not this world, we hand off via /enter.
                    let (local_destination, external_destination) = match Did::try_from(destination.as_str()) {
                        Ok(did) => {
                            if self.is_local_world_root(&did.without_fragment().id()).await {
                                (did.fragment.clone(), None)
                            } else {
                                (None, Some(did.id()))
                            }
                        }
                        Err(_) => (Some(destination.clone()), None),
                    };

                    let mut rooms = self.rooms.write().await;
                    let avatar = rooms
                        .get_mut(room_name)
                        .and_then(|r| r.avatars.remove(from));
                    if let Some(avatar) = avatar {
                        if let Some(external_did) = external_destination {
                            if let Some(src) = rooms.get_mut(room_name) {
                                src.add_avatar(avatar);
                            }
                            let base = travel_text
                                .clone()
                                .unwrap_or_else(|| format!("{} goes {}.", from, exit_name));
                            return (format!("{} go {}", base, external_did), room_name.to_string());
                        }

                        let Some(local_destination) = local_destination else {
                            if let Some(src) = rooms.get_mut(room_name) {
                                src.add_avatar(avatar);
                            }
                            return (format!("Destination '{}' is not a room DID (missing fragment).", destination), room_name.to_string());
                        };

                        if rooms.contains_key(&local_destination) {
                            rooms.get_mut(&local_destination).unwrap().add_avatar(avatar);
                            let base = travel_text
                                .clone()
                                .unwrap_or_else(|| format!("{} goes {}.", from, exit_name));
                            return (base, local_destination);
                        }

                        // Destination vanished — put avatar back.
                        if let Some(src) = rooms.get_mut(room_name) {
                            src.add_avatar(avatar);
                        }
                        return (format!("Destination '{}' no longer exists.", local_destination), room_name.to_string());
                    }
                    let base = travel_text
                        .unwrap_or_else(|| format!("{} goes {}.", from, exit_name));
                    return (base, room_name.to_string());
                }

                if let Some(rest) = trimmed.strip_prefix("describe ") {
                    let description = normalize_spoken_text(rest).trim().to_string();
                    if description.is_empty() {
                        return ("@avatar describe requires text".to_string(), room_name.to_string());
                    }

                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(room_name) else {
                        return (format!("@here room '{}' not found", room_name), room_name.to_string());
                    };
                    let Some(avatar) = room.avatars.get_mut(from) else {
                        return (format!("@avatar '{}' not found", from), room_name.to_string());
                    };

                    avatar.set_description(description.clone());
                    return (format!("@avatar owner={} desc={}", avatar.owner, description), room_name.to_string());
                }

                if trimmed.eq_ignore_ascii_case("show") || trimmed.eq_ignore_ascii_case("who") {
                    let rooms = self.rooms.read().await;
                    let Some(room) = rooms.get(room_name) else {
                        return (format!("@here room '{}' not found", room_name), room_name.to_string());
                    };
                    let Some(avatar) = room.avatars.get(from) else {
                        return (format!("@avatar '{}' not found", from), room_name.to_string());
                    };
                    return (format!(
                        "@avatar owner={} desc={} acl={} shortcuts={}",
                        avatar.owner,
                        avatar.description_or_default(),
                        avatar.acl.summary(),
                        avatar.object_shortcuts_summary()
                    ), room_name.to_string());
                }

                if trimmed.eq_ignore_ascii_case("language show") || trimmed.eq_ignore_ascii_case("lang show") {
                    let rooms = self.rooms.read().await;
                    let Some(room) = rooms.get(room_name) else {
                        return (format!("@here room '{}' not found", room_name), room_name.to_string());
                    };
                    let Some(avatar) = room.avatars.get(from) else {
                        return (format!("@avatar '{}' not found", from), room_name.to_string());
                    };
                    return (
                        format!("@avatar language={}", avatar.language_order),
                        room_name.to_string(),
                    );
                }

                if let Some(rest) = trimmed
                    .strip_prefix("language ")
                    .or_else(|| trimmed.strip_prefix("lang "))
                {
                    let value = rest.trim();
                    if value.is_empty() {
                        return (
                            "@avatar usage: language <ordered-list> (example: nb_NO; en_UK, en; nn_NO)".to_string(),
                            room_name.to_string(),
                        );
                    }
                    let Some(collapsed) = collapse_world_language_order_strict(value) else {
                        return (
                            format!(
                                "@avatar language rejected. supported={}. Set language in closet, or leave.",
                                supported_world_languages_text()
                            ),
                            room_name.to_string(),
                        );
                    };
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(room_name) else {
                        return (format!("@here room '{}' not found", room_name), room_name.to_string());
                    };
                    let Some(avatar) = room.avatars.get_mut(from) else {
                        return (format!("@avatar '{}' not found", from), room_name.to_string());
                    };
                    avatar.language_order = collapsed.clone();
                    return (
                        format!("@avatar language set to {}", collapsed),
                        room_name.to_string(),
                    );
                }

                // Unqualified input is gameplay-first: unknown avatar commands are treated as room commands.
                (
                    self
                        .room_command(
                            room_name,
                            trimmed,
                            from,
                            sender_profile,
                            Some(caller_root_did.as_str()),
                        )
                        .await,
                    room_name.to_string(),
                )
            }
        }
    }

    async fn handle_world_command(
        &self,
        room_name: &str,
        _from: &str,
        from_did: &Did,
        sender_profile: &str,
        command: &str,
    ) -> String {
        let normalized = command.trim();
        let active_lang = world_lang_from_profile(sender_profile);

        if normalized.is_empty() || normalized.eq_ignore_ascii_case("help") {
            return tr_world(
                active_lang,
                "world.help.commands",
                "@world commands: help | list | show [did] | describe [did] | claim | lang [show|set <cid>|clear] | private [on|off|status] | knock list [all] | knock accept <id> | knock reject <id> [note] | knock delete <id> | invite <did> [note] | room <name> acl show|open|close|allow <did>|deny <did> | migrate-index | save | load <cid> | dig <direction> [to|til <#dest|did>]",
            );
        }

        let mut parts = normalized.splitn(2, char::is_whitespace);
        let verb = parts.next().unwrap_or_default().to_ascii_lowercase();
        let arg = parts.next().unwrap_or_default().trim().to_string();

        // Command tokens are world/realm-defined and intentionally invariant.
        // Localized input aliases (e.g. "grave" -> "dig") belong in actor/client.

        if verb == "list" {
            let rooms = self.rooms.read().await;
            if rooms.is_empty() {
                return tr_world(active_lang, "world.list.empty", "@world objects: (none)");
            }

            let mut rows: Vec<(String, String)> = rooms
                .iter()
                .map(|(id, room)| (id.clone(), room.title_or_default()))
                .collect();
            rows.sort_by(|left, right| left.0.cmp(&right.0));

            let payload = rows
                .into_iter()
                .map(|(id, title)| format!("{} => {}", id, title))
                .collect::<Vec<_>>()
                .join("\n");
            return format!("@world objects:\n{}", payload);
        }

        // Caller's root DID is directly available from from_did
        let caller_root_did = from_did.without_fragment().id();

        // @@show [did] — world metadata by default, room metadata when room DID is passed.
        if verb == "show" {
            let world_did_root = self
                .local_world_did_root()
                .await
                .unwrap_or_else(|| "(unconfigured)".to_string());
            if arg.is_empty() {
                let owner = self.owner_did.read().await.clone().unwrap_or_else(|| "(none)".to_string());
                let room_count = self.rooms.read().await.len();
                let lang_cid = self.lang_cid.read().await.clone().unwrap_or_else(|| "(none)".to_string());
                return format!("@world did={} owner={} rooms={} lang_cid={}", world_did_root, owner, room_count, lang_cid);
            }

            let target_fragment = match Did::try_from(arg.as_str()) {
                Ok(did) => {
                    if !self.is_local_world_root(&did.without_fragment().id()).await {
                        return format!("@world unknown DID root: {}", did.without_fragment().id());
                    }
                    did.fragment.clone()
                }
                Err(e) => {
                    if arg.contains(':') {
                        return format!("@world invalid DID '{}': {}", arg, e);
                    }
                    let fragment = arg.trim().trim_start_matches('#');
                    if fragment.is_empty() {
                        return "@world usage: @@show [did:ma:<world>#fragment|fragment]".to_string();
                    }
                    Some(fragment.to_string())
                }
            };

            let Some(fragment) = target_fragment else {
                let owner = self.owner_did.read().await.clone().unwrap_or_else(|| "(none)".to_string());
                let room_count = self.rooms.read().await.len();
                let lang_cid = self.lang_cid.read().await.clone().unwrap_or_else(|| "(none)".to_string());
                return format!("@world did={} owner={} rooms={} lang_cid={}", world_did_root, owner, room_count, lang_cid);
            };

            let rooms = self.rooms.read().await;
            let Some(room) = rooms.get(&fragment) else {
                return format!("@world room '{}' not found", fragment);
            };
            let owner = room.acl.owner.clone().unwrap_or_else(|| "(none)".to_string());
            let room_cid = self
                .room_cids
                .read()
                .await
                .get(&fragment)
                .cloned()
                .unwrap_or_else(|| "(unknown)".to_string());
            return format!(
                "@world room='{}' did={} owner={} cid={} avatars={} exits={}",
                room.name,
                room.did,
                owner,
                room_cid,
                room.avatars.len(),
                room.exits.len()
            );
        }

        // @@describe [did] — world description by default, room description when room DID is passed.
        if verb == "describe" {
            if arg.is_empty() {
                return "@world (no description)".to_string();
            }

            let target_fragment = match Did::try_from(arg.as_str()) {
                Ok(did) => {
                    if !self.is_local_world_root(&did.without_fragment().id()).await {
                        return format!("@world unknown DID root: {}", did.without_fragment().id());
                    }
                    did.fragment.clone()
                }
                Err(e) => {
                    if arg.contains(':') {
                        return format!("@world invalid DID '{}': {}", arg, e);
                    }
                    let fragment = arg.trim().trim_start_matches('#');
                    if fragment.is_empty() {
                        return "@world usage: @@describe [did:ma:<world>#fragment|fragment]".to_string();
                    }
                    Some(fragment.to_string())
                }
            };

            let Some(fragment) = target_fragment else {
                return "@world (no description)".to_string();
            };

            let rooms = self.rooms.read().await;
            let Some(room) = rooms.get(&fragment) else {
                return format!("@world room '{}' not found", fragment);
            };
            return format!("@world {} — {}", room.name, room.description_or_default());
        }

        if verb == "private" {
            let mode = arg.to_ascii_lowercase();
            if mode.is_empty() || mode == "status" {
                if self.is_private_world().await {
                    return tr_world(
                        active_lang,
                        "world.private.status.on",
                        "@world private=on (new entrants must knock)",
                    );
                }
                return tr_world(
                    active_lang,
                    "world.private.status.off",
                    "@world private=off (open entry)",
                );
            }
        }

        // @@claim — set world owner to caller DID if unclaimed.
        if verb == "claim" {
            let current_owner = self.owner_did.read().await.clone();
            if let Some(owner) = current_owner {
                if owner == caller_root_did {
                    return format!("@world already claimed by {}", owner);
                }
                return format!("@world already claimed by {}", owner);
            }

            {
                let mut owner = self.owner_did.write().await;
                *owner = Some(caller_root_did.clone());
            }
            self.allow_entry_did(&caller_root_did).await;
            info!("World claimed by {}", caller_root_did);
            return format!("@world claimed by {}", caller_root_did);
        }

        // All remaining commands require world-owner privilege.
        let owner_did = self.owner_did.read().await.clone();
        let is_owner = owner_did
            .as_ref()
            .map(|owner| owner == &caller_root_did)
            .unwrap_or(false);

        if !is_owner {
            return tr_world(
                active_lang,
                "world.owner.required",
                "@world only the world owner can run that command.",
            );
        }

        if verb == "lang" {
            let trimmed = arg.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("show") {
                let value = self.lang_cid.read().await.clone().unwrap_or_else(|| "(none)".to_string());
                return tr_world_vars(active_lang, "world.lang.show", &[("cid", value.clone())], &format!("@world lang_cid={}", value));
            }
            if trimmed.eq_ignore_ascii_case("clear") {
                self.set_lang_cid(None).await;
                return tr_world(
                    active_lang,
                    "world.lang.cleared",
                    "@world lang_cid cleared (save world to persist)",
                );
            }
            if let Some(cid) = trimmed.strip_prefix("set ") {
                let candidate = cid.trim();
                if candidate.is_empty() {
                    return tr_world(active_lang, "world.lang.usage.set", "@world usage: @@lang set <cid>");
                }
                self.set_lang_cid(Some(candidate.to_string())).await;
                return tr_world_vars(
                    active_lang,
                    "world.lang.set",
                    &[("cid", candidate.to_string())],
                    &format!("@world lang_cid set to {} (save world to persist)", candidate),
                );
            }
            return tr_world(
                active_lang,
                "world.lang.usage",
                "@world usage: @@lang [show|set <cid>|clear]",
            );
        }

        if verb == "private" {
            let mode = arg.to_ascii_lowercase();
            if mode == "on" || mode == "true" {
                self.set_private_world(true).await;
                return tr_world(
                    active_lang,
                    "world.private.on",
                    "@world private mode enabled; new entrants must knock",
                );
            }
            if mode == "off" || mode == "false" {
                self.set_private_world(false).await;
                return tr_world(
                    active_lang,
                    "world.private.off",
                    "@world private mode disabled; entry is now open",
                );
            }
            return tr_world(
                active_lang,
                "world.private.usage",
                "@world usage: @@private [on|off|status]",
            );
        }

        if verb == "knock" {
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("list").to_ascii_lowercase();
            if sub == "list" {
                let include_all = parts.next().map(|v| v.eq_ignore_ascii_case("all")).unwrap_or(false);
                let items = self.list_knocks(!include_all).await;
                if items.is_empty() {
                    return tr_world(active_lang, "world.knock.empty", "@world knock inbox is empty");
                }
                let mut lines = Vec::new();
                for item in items {
                    lines.push(format!(
                        "id={} status={:?} room={} did={} at={}",
                        item.id,
                        item.status,
                        item.room,
                        item.requester_did,
                        item.requested_at
                    ));
                }
                return format!("@world knock inbox:\n{}", lines.join("\n"));
            }

            if sub == "accept" {
                let Some(id_raw) = parts.next() else {
                    return "@world usage: @@knock accept <id>".to_string();
                };
                let id = match Self::parse_knock_id_arg(id_raw) {
                    Ok(value) => value,
                    Err(err) => return format!("@world {}", err),
                };
                return match self.accept_knock(id).await {
                    Ok(item) => format!(
                        "@world knock accepted id={} did={} room={} (entry allowlist updated)",
                        item.id, item.requester_did, item.room
                    ),
                    Err(err) => format!("@world knock accept failed: {}", err),
                };
            }

            if sub == "reject" {
                let Some(id_raw) = parts.next() else {
                    return "@world usage: @@knock reject <id> [note]".to_string();
                };
                let id = match Self::parse_knock_id_arg(id_raw) {
                    Ok(value) => value,
                    Err(err) => return format!("@world {}", err),
                };
                let note = {
                    let rest = parts.collect::<Vec<_>>().join(" ");
                    if rest.trim().is_empty() {
                        None
                    } else {
                        Some(rest)
                    }
                };
                return match self.reject_knock(id, note).await {
                    Ok(item) => format!(
                        "@world knock rejected id={} did={} room={}",
                        item.id, item.requester_did, item.room
                    ),
                    Err(err) => format!("@world knock reject failed: {}", err),
                };
            }

            if sub == "delete" {
                let Some(id_raw) = parts.next() else {
                    return "@world usage: @@knock delete <id>".to_string();
                };
                let id = match id_raw.parse::<u64>() {
                    Ok(value) => value,
                    Err(_) => return format!("@world invalid knock id '{}'", id_raw),
                };
                return match self.delete_knock(id).await {
                    Ok(()) => format!("@world knock deleted id={}", id),
                    Err(err) => format!("@world knock delete failed: {}", err),
                };
            }

            return "@world usage: @@knock list [all] | @@knock accept <id> | @@knock reject <id> [note] | @@knock delete <id>"
                .to_string();
        }

        if verb == "invite" {
            let mut parts = arg.split_whitespace();
            let Some(target_did_raw) = parts.next() else {
                return "@world usage: @@invite <did> [note]".to_string();
            };

            let target_root = match Self::parse_invite_root_did_arg(target_did_raw) {
                Ok(root) => root,
                Err(err) => return format!("@world {}", err),
            };

            let invite_note = {
                let rest = parts.collect::<Vec<_>>().join(" ");
                if rest.trim().is_empty() {
                    "Your knock request was accepted. You may enter now.".to_string()
                } else {
                    rest
                }
            };

            self.allow_entry_did(&target_root).await;
            return format!(
                "@world invited {} (allowlisted). note='{}'",
                target_root,
                invite_note
            );
        }

        if verb == "migrate-index" {
            let room_names = {
                let rooms = self.rooms.read().await;
                let mut names = rooms.keys().cloned().collect::<Vec<_>>();
                names.sort();
                names
            };

            if room_names.is_empty() {
                return "@world migrate-index: no rooms to persist".to_string();
            }

            match self.save_rooms_and_world_index(&room_names).await {
                Ok(new_cid) => {
                    return format!(
                        "@world migrate-index complete: root_cid={} rooms={}",
                        new_cid,
                        room_names.len()
                    );
                }
                Err(e) => {
                    return format!("@world migrate-index failed: {}", e);
                }
            }
        }

        if verb == "save" {
            match self.save_encrypted_state().await {
                Ok((state_cid, root_cid)) => {
                    return format!(
                        "@world saved encrypted runtime state: state_cid={} root_cid={}",
                        state_cid, root_cid
                    );
                }
                Err(e) => {
                    return format!("@world save failed: {}", e);
                }
            }
        }

        if verb == "load" {
            if arg.is_empty() {
                return "@world usage: @@load <cid>".to_string();
            }
            match self.load_encrypted_state(arg.as_str()).await {
                Ok(root_cid) => {
                    return format!(
                        "@world loaded encrypted runtime state from {} (root_cid={})",
                        arg, root_cid
                    );
                }
                Err(e) => {
                    return format!("@world load failed: {}", e);
                }
            }
        }

        if verb == "dig" {
            if arg.is_empty() {
                return "@world usage: @@dig <direction> [to|til <#dest|did:ma:...#room>]".to_string();
            }

            let (exit_name, destination) = if let Some((dir, dest)) = arg
                .split_once(" to ")
                .or_else(|| arg.split_once(" til "))
            {
                let dest_clean = dest.trim().trim_start_matches('#').to_string();
                (dir.trim().to_string(), if dest_clean.is_empty() { None } else { Some(dest_clean) })
            } else {
                (arg.clone(), None)
            };

            let destination_input = destination
                .clone()
                .unwrap_or_else(|| nanoid!());
            let exit_target: String;
            let mut local_room_to_create: Option<String> = None;

            match Did::try_from(destination_input.as_str()) {
                Ok(did) => {
                    if self.is_local_world_root(&did.without_fragment().id()).await {
                        let Some(fragment) = did.fragment.clone() else {
                            return "@world usage: @@dig <direction> [to <#dest|did:ma:...#room>]".to_string();
                        };
                        exit_target = fragment.clone();
                        local_room_to_create = Some(fragment);
                    } else {
                        exit_target = did.id();
                    }
                }
                Err(e) => {
                    if destination_input.contains(':') {
                        return format!("@world invalid destination DID '{}': {}", destination_input, e);
                    }
                    let local_id = normalize_local_object_id(&destination_input);
                    if !is_valid_nanoid_id(&local_id) {
                        return format!(
                            "@world invalid destination id '{}': expected nanoid-compatible id ([A-Za-z0-9_-]+)",
                            destination_input
                        );
                    }
                    exit_target = local_id.clone();
                    local_room_to_create = Some(local_id);
                }
            }

            let exit_id = format!("{}-{}", room_name, exit_name);
            let mut changed_rooms: Vec<String> = vec![room_name.to_string()];

            let mut rooms = self.rooms.write().await;
            if let Some(local_room) = local_room_to_create.clone() {
                let room_did = self.build_room_did(&local_room).await;
                rooms
                    .entry(local_room.clone())
                    .or_insert_with(|| crate::room::Room::new(local_room.clone(), room_did));
                changed_rooms.push(local_room);
            }
            if let Some(room) = rooms.get_mut(room_name) {
                if !room.exits.iter().any(|e| e.matches(&exit_name)) {
                    room.exits.push(build_exit_entry(exit_id, exit_name.clone(), exit_target.clone()));
                }
            }
            drop(rooms);

            if let Err(e) = self.save_rooms_and_world_index(&changed_rooms).await {
                warn!(
                    "Failed to persist world dig room snapshots for {:?}: {}",
                    changed_rooms,
                    e
                );
            }
            return format!("@world exit '{}' dug from '{}' → '{}'", exit_name, room_name, exit_target);
        }

        if verb == "room" {
            // @@room <name> acl show|open|close|allow <did>|deny <did>
            // World-owner admin override for room-level ACLs.
            // Does NOT automatically bypass the ACL — caller must change it explicitly.
            let mut room_parts = arg.splitn(3, char::is_whitespace);
            let room_name_arg = room_parts.next().unwrap_or_default().trim().to_string();
            let sub = room_parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let sub_arg = room_parts.next().unwrap_or_default().trim().to_string();

            if room_name_arg.is_empty() || sub != "acl" {
                return "@world usage: @@room <name> acl show|open|close|allow <did>|deny <did>".to_string();
            }

            let mut acl_parts = sub_arg.splitn(2, char::is_whitespace);
            let acl_cmd = acl_parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let acl_arg = acl_parts.next().unwrap_or_default().trim().to_string();

            match acl_cmd.as_str() {
                "" | "show" => {
                    let rooms = self.rooms.read().await;
                    let Some(room) = rooms.get(&room_name_arg) else {
                        return format!("@world room '{}' not found", room_name_arg);
                    };
                    return format!(
                        "@world room '{}' acl: {} owner={}",
                        room_name_arg,
                        room.acl.summary(),
                        room.acl.owner.as_deref().unwrap_or("(none)")
                    );
                }
                "open" => {
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return format!("@world room '{}' not found", room_name_arg);
                    };
                    room.acl.allow.insert("*".to_string());
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return format!("@world room '{}' acl opened (public)", room_name_arg);
                }
                "close" => {
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return format!("@world room '{}' not found", room_name_arg);
                    };
                    room.acl.allow.remove("*");
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return format!("@world room '{}' acl closed (private)", room_name_arg);
                }
                "allow" => {
                    if acl_arg.is_empty() {
                        return format!("@world usage: @@room {} acl allow <did>", room_name_arg);
                    }
                    let target_root = match Did::try_from(acl_arg.as_str()) {
                        Ok(d) => d.without_fragment().id(),
                        Err(e) => return format!("@world invalid DID '{}': {}", acl_arg, e),
                    };
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return format!("@world room '{}' not found", room_name_arg);
                    };
                    room.acl.allow.insert(target_root.clone());
                    room.acl.deny.remove(&target_root);
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return format!("@world room '{}' acl: allowed {}", room_name_arg, target_root);
                }
                "deny" => {
                    if acl_arg.is_empty() {
                        return format!("@world usage: @@room {} acl deny <did>", room_name_arg);
                    }
                    let target_root = match Did::try_from(acl_arg.as_str()) {
                        Ok(d) => d.without_fragment().id(),
                        Err(e) => return format!("@world invalid DID '{}': {}", acl_arg, e),
                    };
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return format!("@world room '{}' not found", room_name_arg);
                    };
                    if room.acl.owner.as_deref() == Some(target_root.as_str()) {
                        return format!("@world room '{}' owner cannot be denied", room_name_arg);
                    }
                    room.acl.deny.insert(target_root.clone());
                    room.acl.allow.remove(&target_root);
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return format!("@world room '{}' acl: denied {}", room_name_arg, target_root);
                }
                _ => {
                    return format!(
                        "@world unknown acl subcommand '{}'. usage: @@room {} acl show|open|close|allow <did>|deny <did>",
                        acl_cmd, room_name_arg
                    );
                }
            }
        }

        format!("@world unknown command: {}", normalized)
    }

    async fn room_command(
        &self,
        room_name: &str,
        command: &str,
        from: &str,
        _sender_profile: &str,
        caller_root_did: Option<&str>,
    ) -> String {

        let (room_exists, avatars, acl_owner, acl_summary, description, did) = {
            let rooms = self.rooms.read().await;
            if let Some(room) = rooms.get(room_name) {
                (
                    true,
                    room.avatars.keys().cloned().collect::<Vec<_>>(),
                    room.acl.owner.clone(),
                    room.acl.summary(),
                    room.description_or_default(),
                    Some(room.did.clone()),
                )
            } else {
                (false, Vec::new(), None, "(none)".to_string(), String::new(), None)
            }
        };
        let things = self.room_object_names(room_name).await;

        let ctx = RoomActorContext {
            room_name,
            room_exists,
            avatars,
            things,
            acl_owner_did: acl_owner.as_deref(),
            acl_summary: &acl_summary,
            caller_root_did,
            description: &description,
            did: did.as_deref(),
        };

        let trimmed = command.trim();
        if let Some(rest) = trimmed.strip_prefix("id ") {
            let token = rest.trim().trim_start_matches('@');
            if token.is_empty() {
                return "@here usage: @here id <thing|avatar|room|me>".to_string();
            }
            if token.eq_ignore_ascii_case("here") || token.eq_ignore_ascii_case("room") {
                return did
                    .clone()
                    .map(|value| format!("did={} source=room room={}", value, room_name))
                    .unwrap_or_else(|| "@here room DID unavailable".to_string());
            }
            if token.eq_ignore_ascii_case("me") || token.eq_ignore_ascii_case("avatar") {
                if let Some(root) = caller_root_did {
                    return format!("did={} source=caller handle={}", root, from);
                }
                return "@here caller DID unavailable".to_string();
            }
            if let Some(object_id) = self.resolve_room_object_id(room_name, token).await {
                let world_did_root = self
                    .local_world_did_root()
                    .await
                    .unwrap_or_else(|| "did:ma:unconfigured".to_string());
                return format!(
                    "did={}#{} source=object room={} object_id={} token={}",
                    world_did_root,
                    object_id,
                    room_name,
                    object_id,
                    token
                );
            }
            let rooms = self.rooms.read().await;
            if let Some(room) = rooms.get(room_name) {
                if let Some(avatar) = room.avatars.get(token) {
                    return format!(
                        "did={} source=avatar handle={}",
                        avatar.agent_did.without_fragment().id(),
                        token
                    );
                }
            }
            return format!("@here id '{}' not found", token);
        }

        let decision = execute_room_actor_command(command, &ctx);
        let mut response = decision.response.clone();
        let mut changed_rooms: Vec<String> = Vec::new();
        let mut room_update_announcement: Option<String> = None;

        match decision.action {
            RoomActorAction::None => {}
            RoomActorAction::Invite { did } => {
                let mut rooms = self.rooms.write().await;
                if let Some(room) = rooms.get_mut(room_name) {
                    room.acl.allow.insert(did.clone());
                    room.acl.deny.remove(&did);
                    changed_rooms.push(room_name.to_string());
                }
            }
            RoomActorAction::Deny { did } => {
                let mut rooms = self.rooms.write().await;
                if let Some(room) = rooms.get_mut(room_name) {
                    if room.acl.owner.as_deref() == Some(did.as_str()) {
                        response = "@here owner cannot be denied".to_string();
                    } else {
                        room.acl.deny.insert(did.clone());
                        room.acl.allow.remove(&did);
                        room.avatars.retain(|_, av| av.agent_did.without_fragment().id() != did);
                        changed_rooms.push(room_name.to_string());
                    }
                }
            }
            RoomActorAction::Kick { handle } => {
                let mut rooms = self.rooms.write().await;
                if let Some(room) = rooms.get_mut(room_name) {
                    room.avatars.remove(&handle);
                }
            }
            RoomActorAction::SetAttribute { key, value } => {
                match key.as_str() {
                    "owner" => {
                        let did = match Did::try_from(value.as_str()) {
                            Ok(d) => d.without_fragment().id(),
                            Err(e) => {
                                response = format!("@here invalid owner DID '{}': {}", value, e);
                                return response;
                            }
                        };
                        let mut rooms = self.rooms.write().await;
                        if let Some(room) = rooms.get_mut(room_name) {
                            room.acl.owner = Some(did.clone());
                            room.acl.allow.insert(did.clone());
                            room.acl.deny.remove(&did);
                            changed_rooms.push(room_name.to_string());
                        }
                    }
                    "title" => {
                        let title_value = value.clone();
                        let mut rooms = self.rooms.write().await;
                        if let Some(room) = rooms.get_mut(room_name) {
                            room.set_title(title_value);
                            changed_rooms.push(room_name.to_string());
                            room_update_announcement = Some(format!(
                                "room title updated by {}",
                                from
                            ));
                        }
                    }
                    "description" => {
                        let description_value = value.clone();
                        let mut rooms = self.rooms.write().await;
                        if let Some(room) = rooms.get_mut(room_name) {
                            room.set_description(description_value);
                            changed_rooms.push(room_name.to_string());
                            room_update_announcement = Some(format!(
                                "room description updated by {}",
                                from
                            ));
                        }
                    }
                    "cid" => {
                        let (cid, yaml_text, published_from_yaml) = match self.resolve_room_cid_or_yaml_input(&value).await {
                            Ok(tuple) => tuple,
                            Err(err) => {
                                response = format!("@here invalid room payload: {}", err);
                                return response;
                            }
                        };

                        match self.materialize_room_from_yaml(room_name, &yaml_text).await {
                            Err(e) => {
                                response = format!("@here invalid room YAML payload: {}", e);
                            }
                            Ok((mut loaded, _needs_rewrite)) => {
                                {
                                    // Preserve runtime avatars from the current room.
                                    let mut rooms = self.rooms.write().await;
                                    if let Some(existing) = rooms.get(room_name) {
                                        loaded.avatars = existing.avatars.clone();
                                    }
                                    let new_owner = loaded.acl.owner.clone().unwrap_or_else(|| "(none)".to_string());
                                    if published_from_yaml {
                                        response = format!(
                                            "@here room '{}' content published and applied as {} (owner: {})",
                                            room_name,
                                            cid,
                                            new_owner
                                        );
                                    } else {
                                        response = format!("@here room '{}' replaced from {} (owner: {})", room_name, cid, new_owner);
                                    }
                                    rooms.insert(room_name.to_string(), loaded);
                                }
                                self.room_cids.write().await.insert(room_name.to_string(), cid.clone());
                                if let Err(e) = self.save_world_index().await {
                                    warn!("Failed to save world index after set cid/content: {}", e);
                                }
                            }
                        }
                    }
                    "content" | "content-b64" => {
                        let (cid, yaml_text, _published_from_yaml) = match self.resolve_room_cid_or_yaml_input(&value).await {
                            Ok(tuple) => tuple,
                            Err(err) => {
                                response = format!("@here invalid room payload: {}", err);
                                return response;
                            }
                        };

                        match self.materialize_room_from_yaml(room_name, &yaml_text).await {
                            Err(err) => {
                                response = format!("@here invalid room YAML payload: {}", err);
                            }
                            Ok((mut loaded, _needs_rewrite)) => {
                                {
                                    let mut rooms = self.rooms.write().await;
                                    if let Some(existing) = rooms.get(room_name) {
                                        loaded.avatars = existing.avatars.clone();
                                    }
                                    rooms.insert(room_name.to_string(), loaded);
                                }
                                self.room_cids
                                    .write()
                                    .await
                                    .insert(room_name.to_string(), cid.clone());
                                if let Err(e) = self.save_world_index().await {
                                    warn!("Failed to save world index after set content-b64: {}", e);
                                }
                                response = format!(
                                    "@here room '{}' content published and applied as {}",
                                    room_name,
                                    cid
                                );
                            }
                        }
                    }
                        "exit-content-b64" => {
                            let mut parts = value.splitn(2, char::is_whitespace);
                            let exit_id = parts.next().unwrap_or_default().trim();
                            let encoded = parts.next().unwrap_or_default().trim();
                            if exit_id.is_empty() || encoded.is_empty() {
                                response = "@here usage: @here set exit-content-b64 <exit-id> <base64-yaml>".to_string();
                                return response;
                            }

                            let decoded = match B64.decode(encoded.as_bytes()) {
                                Ok(bytes) => bytes,
                                Err(err) => {
                                    response = format!("@here invalid base64 exit content: {}", err);
                                    return response;
                                }
                            };
                            let exit_yaml = match String::from_utf8(decoded) {
                                Ok(text) => text,
                                Err(err) => {
                                    response = format!("@here invalid UTF-8 exit YAML payload: {}", err);
                                    return response;
                                }
                            };

                            if let Err(err) = serde_yaml::from_str::<ExitYamlDoc>(&exit_yaml) {
                                response = format!("@here invalid exit YAML payload: {}", err);
                                return response;
                            }

                            let kubo_url = self.kubo_url().await;
                            let new_exit_cid = match ipfs_add(&kubo_url, exit_yaml.into_bytes()).await {
                                Ok(cid) => cid,
                                Err(err) => {
                                    response = format!("@here failed to publish exit YAML: {}", err);
                                    return response;
                                }
                            };

                            let current_room_cid = {
                                let room_cids = self.room_cids.read().await;
                                room_cids.get(room_name).cloned()
                            };
                            let Some(current_room_cid) = current_room_cid else {
                                response = "@here room has no published CID yet; use @here set content-b64 for full room YAML first".to_string();
                                return response;
                            };

                            let current_room_yaml = match kubo::cat_cid(&kubo_url, &current_room_cid).await {
                                Ok(text) => text,
                                Err(err) => {
                                    response = format!(
                                        "@here failed to load current room CID {}: {}",
                                        current_room_cid,
                                        err
                                    );
                                    return response;
                                }
                            };

                            let mut room_doc = match serde_yaml::from_str::<RoomYamlDocV2>(&current_room_yaml) {
                                Ok(doc) => doc,
                                Err(err) => {
                                    response = format!(
                                        "@here current room YAML at {} is not editable as v2 content: {}",
                                        current_room_cid,
                                        err
                                    );
                                    return response;
                                }
                            };

                            room_doc.exit_cids.insert(exit_id.to_string(), new_exit_cid.clone());
                            room_doc.exits.clear();

                            let updated_room_yaml = match serde_yaml::to_string(&room_doc) {
                                Ok(text) => text,
                                Err(err) => {
                                    response = format!("@here failed to encode updated room YAML: {}", err);
                                    return response;
                                }
                            };

                            let updated_room_cid = match ipfs_add(&kubo_url, updated_room_yaml.as_bytes().to_vec()).await {
                                Ok(cid) => cid,
                                Err(err) => {
                                    response = format!("@here failed to publish updated room YAML: {}", err);
                                    return response;
                                }
                            };

                            match self.materialize_room_from_yaml(room_name, &updated_room_yaml).await {
                                Err(err) => {
                                    response = format!("@here invalid updated room YAML payload: {}", err);
                                }
                                Ok((mut loaded, _needs_rewrite)) => {
                                    {
                                        let mut rooms = self.rooms.write().await;
                                        if let Some(existing) = rooms.get(room_name) {
                                            loaded.avatars = existing.avatars.clone();
                                        }
                                        rooms.insert(room_name.to_string(), loaded);
                                    }
                                    self.room_cids
                                        .write()
                                        .await
                                        .insert(room_name.to_string(), updated_room_cid.clone());
                                    if let Err(e) = self.save_world_index().await {
                                        warn!("Failed to save world index after set exit-content-b64: {}", e);
                                    }
                                    response = format!(
                                        "@here exit '{}' published as {} and room '{}' updated to {}",
                                        exit_id,
                                        new_exit_cid,
                                        room_name,
                                        updated_room_cid
                                    );
                                }
                            }
                        }
                    _ => {
                        response = format!("@here unknown set attribute '{}'.", key);
                    }
                }
            }
            RoomActorAction::Dig { exit_name, destination } => {
                let destination_input = destination
                    .unwrap_or_else(|| nanoid!());
                let exit_target: String;
                let mut local_room_to_create: Option<String> = None;

                match Did::try_from(destination_input.as_str()) {
                    Ok(did) => {
                        if self.is_local_world_root(&did.without_fragment().id()).await {
                            let Some(fragment) = did.fragment.clone() else {
                                response = "@here usage: @here dig <direction> [to <#dest|did:ma:...#room>]".to_string();
                                return response;
                            };
                            exit_target = fragment.clone();
                            local_room_to_create = Some(fragment);
                        } else {
                            exit_target = did.id();
                        }
                    }
                    Err(e) => {
                        if destination_input.contains(':') {
                            response = format!("@here invalid destination DID '{}': {}", destination_input, e);
                            return response;
                        }
                        let local_id = normalize_local_object_id(&destination_input);
                        if !is_valid_nanoid_id(&local_id) {
                            response = format!(
                                "@here invalid destination id '{}': expected nanoid-compatible id ([A-Za-z0-9_-]+)",
                                destination_input
                            );
                            return response;
                        }
                        exit_target = local_id.clone();
                        local_room_to_create = Some(local_id);
                    }
                }

                let exit_id = format!("{}-{}", room_name, exit_name);
                let mut rooms = self.rooms.write().await;
                // Create the destination room if it doesn't exist yet.
                if let Some(local_room) = local_room_to_create.clone() {
                    if !rooms.contains_key(&local_room) {
                        let room_did = self.build_room_did(&local_room).await;
                        let mut room = crate::room::Room::new(local_room.clone(), room_did);
                        if let Some(caller) = caller_root_did {
                            room.acl.owner = Some(caller.to_string());
                        }
                        rooms.insert(local_room, room);
                    }
                }
                // Add the outbound exit to the source room.
                if let Some(room) = rooms.get_mut(room_name) {
                    let already_exists = room.exits.iter().any(|e| e.matches(&exit_name));
                    if !already_exists {
                        room.exits.push(build_exit_entry(exit_id, exit_name.clone(), exit_target));
                    }
                }
                changed_rooms.push(room_name.to_string());
                if let Some(created_room) = local_room_to_create {
                    changed_rooms.push(created_room);
                }
                room_update_announcement = Some(format!("new exit '{}' created by {}", exit_name, from));
            }
        }

        if !changed_rooms.is_empty() {
            if let Err(e) = self.save_rooms_and_world_index(&changed_rooms).await {
                warn!(
                    "Failed to persist changed room snapshots for {:?}: {}",
                    changed_rooms,
                    e
                );
            }
        }

        if let Some(message) = room_update_announcement {
            self.record_room_event(
                room_name,
                "room.update",
                Some(from.to_string()),
                caller_root_did.map(|v| v.to_string()),
                None,
                message,
            )
            .await;
        }

        response
    }

    async fn record_event(&self, event: String) {
        let entry = format!("{} {}", Utc::now().to_rfc3339(), event);
        let mut events = self.events.write().await;
        if events.len() >= MAX_EVENTS {
            events.pop_front();
        }
        events.push_back(entry);
    }

    async fn record_room_event(
        &self,
        room_name: &str,
        kind: &str,
        sender: Option<String>,
        sender_did: Option<String>,
        sender_endpoint: Option<String>,
        message: String,
    ) -> u64 {
        let mut next_sequence = self.next_room_event_sequence.write().await;
        *next_sequence += 1;
        let sequence = *next_sequence;
        drop(next_sequence);

        let entry = RoomEvent {
            sequence,
            room: room_name.to_string(),
            kind: kind.to_string(),
            sender,
            sender_did,
            sender_endpoint,
            message,
            message_cbor_b64: None,
            occurred_at: Utc::now().to_rfc3339(),
        };

        let mut room_events = self.room_events.write().await;
        let events = room_events
            .entry(room_name.to_string())
            .or_insert_with(|| VecDeque::with_capacity(MAX_EVENTS));
        if events.len() >= MAX_EVENTS {
            events.pop_front();
        }
        events.push_back(entry);
        sequence
    }
}

impl WorldProtocol {
    fn content_type_matches(actual: &str, canonical: &str, legacy: &str) -> bool {
        actual == canonical || actual == legacy
    }

    async fn room_signing_key(&self, room_did: &str) -> Result<SigningKey> {
        let slots = self.world.actor_secrets.read().await;
        let Some(secret) = slots.get(room_did) else {
            return Err(anyhow!("missing room actor secret for {}", room_did));
        };

        let room_root = Did::try_from(room_did)
            .map_err(|e| anyhow!("invalid room did '{}': {}", room_did, e))?
            .without_fragment()
            .id();
        let signing_did = Did::try_from(room_root.as_str())
            .map_err(|e| anyhow!("invalid room root did '{}': {}", room_root, e))?
            .with_fragment("sig")
            .map_err(|e| anyhow!("invalid signing did for room {}: {}", room_did, e))?;
        SigningKey::from_private_key_bytes(signing_did, secret.signing_key)
            .map_err(|e| anyhow!("failed to restore signing key for {}: {}", room_did, e))
    }

    async fn room_presence_context(
        &self,
        room_name: &str,
    ) -> Result<(String, String, String, Vec<PresenceAvatar>, Vec<String>)> {
        let rooms = self.world.rooms.read().await;
        let room = rooms
            .get(room_name)
            .ok_or_else(|| anyhow!("room '{}' not found", room_name))?;

        let mut avatars = Vec::new();
        let mut endpoints = Vec::new();
        for (handle, avatar) in &room.avatars {
            avatars.push(PresenceAvatar {
                handle: handle.clone(),
                did: avatar.agent_did.id(),
            });
            endpoints.push(avatar.agent_endpoint.clone());
        }
        avatars.sort_by(|a, b| a.handle.cmp(&b.handle));
        endpoints.sort();
        endpoints.dedup();

        Ok((
            room.did.clone(),
            room.title_or_default(),
            room.description_or_default(),
            avatars,
            endpoints,
        ))
    }

    async fn send_signed_push_to_endpoint_on_lane(
        &self,
        target_endpoint_id: &str,
        message_cbor: Vec<u8>,
        lane_alpn: &'static [u8],
    ) -> Result<()> {
        let target: EndpointId = target_endpoint_id
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid target endpoint id {}: {}", target_endpoint_id, e))?;

        let relay: RelayUrl = DEFAULT_WORLD_RELAY_URL
            .parse()
            .map_err(|e| anyhow!("invalid relay URL {}: {}", DEFAULT_WORLD_RELAY_URL, e))?;
        let endpoint_addr = EndpointAddr::new(target).with_relay_url(relay);

        let connection = self
            .endpoint
            .connect(endpoint_addr, lane_alpn)
            .await
            .map_err(|e| anyhow!("push endpoint.connect failed: {}", e))?;

        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| anyhow!("push connection.open_bi failed: {}", e))?;

        let request = OutboxRequest::Signed { message_cbor };
        let payload = serde_json::to_vec(&request)
            .map_err(|e| anyhow!("failed to serialize outbox request: {}", e))?;

        send.write_u32(payload.len() as u32)
            .await
            .map_err(|e| anyhow!("push write_u32 failed: {}", e))?;
        send.write_all(&payload)
            .await
            .map_err(|e| anyhow!("push write_all failed: {}", e))?;
        send.flush()
            .await
            .map_err(|e| anyhow!("push flush failed: {}", e))?;

        let frame_len = recv
            .read_u32()
            .await
            .map_err(|e| anyhow!("push read_u32 failed: {}", e))? as usize;
        if frame_len > 256 * 1024 {
            return Err(anyhow!("push response frame too large: {}", frame_len));
        }

        let mut bytes = vec![0u8; frame_len];
        recv.read_exact(&mut bytes)
            .await
            .map_err(|e| anyhow!("push read_exact failed: {}", e))?;
        let response: OutboxResponse = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow!("push response decode failed: {}", e))?;
        if !response.ok {
            return Err(anyhow!("push rejected: {}", response.message));
        }

        let _ = send.finish();
        connection.close(0u32.into(), b"ok");
        Ok(())
    }

    async fn send_signed_push_to_endpoint(
        &self,
        target_endpoint_id: &str,
        message_cbor: Vec<u8>,
    ) -> Result<()> {
        self.send_signed_push_to_endpoint_on_lane(target_endpoint_id, message_cbor, PRESENCE_ALPN)
            .await
    }

    async fn push_presence_snapshot_to(
        &self,
        room_name: &str,
        target_endpoint_id: &str,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_did, room_title, room_description, avatars, _endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence snapshot context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_did).await {
            Ok(key) => key,
            Err(err) => {
                warn!("presence snapshot signing key unavailable for {}: {}", room_did, err);
                return;
            }
        };
        let seq = self
            .world
            .latest_room_event_sequence(room_name)
            .await
            .unwrap_or(0);

        let payload = PresenceSnapshotEvent {
            v: 1,
            kind: "presence.snapshot".to_string(),
            room: room_name.to_string(),
            room_did: room_did.clone(),
            room_title,
            room_description,
            avatars,
            seq,
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence snapshot encode failed for room '{}': {}", room_name, err);
                return;
            }
        };
        let message = match Message::new(
            room_did.clone(),
            room_did,
            CONTENT_TYPE_PRESENCE,
            content,
            &signing_key,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!("presence snapshot message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence snapshot cbor encode failed: {}", err);
                return;
            }
        };

        if let Err(err) = self.send_signed_push_to_endpoint(target_endpoint_id, cbor).await {
            warn!("presence snapshot push to {} failed: {}", target_endpoint_id, err);
        }
    }

    async fn push_presence_snapshot(&self, room_name: &str) {
        let context = self.room_presence_context(room_name).await;
        let (_room_did, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence snapshot context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };

        for endpoint in endpoints {
            self.push_presence_snapshot_to(room_name, &endpoint).await;
        }
    }

    async fn push_room_broadcast(
        &self,
        room_name: &str,
        sender: Option<String>,
        message: String,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_did, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("broadcast context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_did).await {
            Ok(key) => key,
            Err(err) => {
                warn!("broadcast signing key unavailable for {}: {}", room_did, err);
                return;
            }
        };

        let seq = self
            .world
            .latest_room_event_sequence(room_name)
            .await
            .unwrap_or(0);
        let payload = RoomBroadcastEvent {
            v: 1,
            kind: "room.broadcast".to_string(),
            room: room_name.to_string(),
            room_did: room_did.clone(),
            sender,
            message,
            seq,
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("broadcast encode failed for room '{}': {}", room_name, err);
                return;
            }
        };
        let message = match Message::new(
            room_did.clone(),
            room_did,
            CONTENT_TYPE_BROADCAST,
            content,
            &signing_key,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!("broadcast message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("broadcast cbor encode failed: {}", err);
                return;
            }
        };

        for endpoint in endpoints {
            if let Err(err) = self
                .send_signed_push_to_endpoint_on_lane(&endpoint, cbor.clone(), BROADCAST_ALPN)
                .await
            {
                warn!("broadcast push to {} failed: {}", endpoint, err);
            }
        }
    }

    async fn process_request(&self, request: WorldRequest, agent_endpoint: String) -> WorldResponse {
        match self.handle_request(request, agent_endpoint).await {
            Ok(resp) => resp,
            Err(err) => {
                warn!("request error: {}", err);
                let detail = err.to_string();
                let ack_code = if detail.contains("does not support this request type") {
                    TransportAckCode::UnsupportedRequestType
                } else if detail.contains("expected ") && detail.contains(" on this lane") {
                    TransportAckCode::InvalidContentType
                } else {
                    TransportAckCode::Rejected
                };

                WorldResponse {
                    ok: false,
                    room: String::new(),
                    message: detail.clone(),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence: 0,
                    broadcasted: false,
                    events: Vec::new(),
                    handle: String::new(),
                    room_description: String::new(),
                    room_title: String::new(),
                    room_did: String::new(),
                    avatars: Vec::new(),
                    room_object_dids: HashMap::new(),
                    transport_ack: Some(TransportAck {
                        lane: self.lane.label().to_string(),
                        code: ack_code,
                        detail,
                    }),
                }
            }
        }
    }

    async fn get_sender_document(&self, sender_root: &Did, force_refresh: bool) -> Result<(Document, bool, bool)> {
        let cache_key = sender_root.id();

        if !force_refresh {
            let cache = self.did_cache.read().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok((cached.document.clone(), false, cached.dirty));
            }
        }

        let kubo_url = self.world.kubo_url().await;
        let fetched = kubo::fetch_did_document(&kubo_url, sender_root).await?;

        let existing_dirty = {
            let cache = self.did_cache.read().await;
            cache.get(&cache_key).map(|entry| entry.dirty).unwrap_or(false)
        };

        let mut cache = self.did_cache.write().await;
        cache.insert(
            cache_key,
            CachedDidDocument {
                document: fetched.clone(),
                dirty: existing_dirty,
            },
        );

        Ok((fetched, true, existing_dirty))
    }

    async fn set_sender_dirty(&self, sender_root: &Did, dirty: bool) {
        let cache_key = sender_root.id();
        let mut cache = self.did_cache.write().await;
        if let Some(cached) = cache.get_mut(&cache_key) {
            cached.dirty = dirty;
        }
    }

    async fn verify_message(&self, message_cbor: Vec<u8>) -> Result<(Message, Did, Document)> {
        let message = Message::from_cbor(&message_cbor)?;
        let sender_did = Did::try_from(message.from.as_str())?;
        let sender_root = sender_did.without_fragment();

        let t0 = std::time::Instant::now();
        let (sender_document, fetched_from_kubo, is_dirty) = self.get_sender_document(&sender_root, false).await
            .map_err(|e| {
                warn!("DID doc fetch failed for {} after {:?}: {}", sender_root.id(), t0.elapsed(), e);
                e
            })?;
        if fetched_from_kubo {
            info!("DID doc for {} fetched via Kubo in {:?}", sender_root.id(), t0.elapsed());
        } else {
            debug!("DID doc for {} served from cache in {:?}", sender_root.id(), t0.elapsed());
        }
        if is_dirty {
            warn!("DID {} is marked dirty; using cached document", sender_root.id());
        }

        if message.verify_with_document(&sender_document).is_ok() {
            if is_dirty {
                self.set_sender_dirty(&sender_root, false).await;
                info!("DID {} cleared from dirty state after successful verification", sender_root.id());
            }
            return Ok((message, sender_did, sender_document));
        }

        warn!(
            "signature verification failed with cached DID doc for {}; retrying fresh fetch",
            sender_root.id()
        );

        let refresh_t0 = std::time::Instant::now();
        let (refreshed_document, refreshed_from_kubo, _) =
            match self.get_sender_document(&sender_root, true).await {
                Ok(value) => value,
                Err(e) => {
                    self.set_sender_dirty(&sender_root, true).await;
                    warn!(
                        "forced DID doc refetch failed for {} after {:?}: {}",
                        sender_root.id(),
                        refresh_t0.elapsed(),
                        e
                    );
                    return Err(e);
                }
            };
        if refreshed_from_kubo {
            info!(
                "DID doc for {} force-fetched via Kubo in {:?}",
                sender_root.id(),
                refresh_t0.elapsed()
            );
        }

        if message.verify_with_document(&refreshed_document).is_ok() {
            self.set_sender_dirty(&sender_root, false).await;
            return Ok((message, sender_did, refreshed_document));
        }

        self.set_sender_dirty(&sender_root, true).await;
        warn!(
            "DID {} marked dirty: signature verification still failed after forced refresh",
            sender_root.id()
        );

        Err(anyhow!(
            "message signature verification failed for {} (cached and refreshed DID document)",
            sender_root.id()
        ))
    }

    async fn handle_request(&self, request: WorldRequest, agent_endpoint: String) -> Result<WorldResponse> {
        if !self.world.is_unlocked().await {
            return Err(anyhow!(
                "world is locked; unlock from the status page before sending commands"
            ));
        }

        if !self.lane.supports_request(&request) {
            return Err(anyhow!(
                "lane '{}' does not support this request type",
                self.lane.label()
            ));
        }

        match request {
            WorldRequest::Signed { message_cbor } => {
                // Each ALPN lane has exactly one canonical content type.
                let (message, sender_did, sender_document) = self.verify_message(message_cbor).await?;
                let expected_ct = match self.lane {
                    WorldLane::World => CONTENT_TYPE_WORLD,
                    WorldLane::Cmd => CONTENT_TYPE_CMD,
                    _ => return Err(anyhow!("signed request not supported on this lane")),
                };
                if !Self::content_type_matches(&message.content_type, expected_ct, "application/x-ma-command") {
                    return Err(anyhow!("expected {} on this lane, got {}", expected_ct, message.content_type));
                }
                let command: WorldCommand = serde_json::from_slice(&message.content)
                    .map_err(|err| anyhow!("invalid command payload: {}", err))?;
                let sender_root = sender_did.without_fragment().id();
                self.handle_command(command, &message, &sender_did, sender_document, agent_endpoint, &sender_root).await
            }

            WorldRequest::Chat { room, message_cbor } => {
                if self.lane != WorldLane::Chat {
                    return Err(anyhow!("chat is only supported on ma/chat/1"));
                }
                let (message, sender_did, _) = self.verify_message(message_cbor.clone()).await?;
                if !Self::content_type_matches(&message.content_type, CONTENT_TYPE_CHAT, "application/x.ma.chat") {
                    return Err(anyhow!("expected application/x-ma-chat, got {}", message.content_type));
                }
                let sender_root = sender_did.without_fragment().id();
                let handle = self.world.resolve_handle(&sender_root).await;
                if handle == sender_root {
                    return Err(anyhow!("not registered in room — enter first"));
                }
                self.world.send_chat(&room, &handle, &sender_root, message_cbor).await?;
                let latest_event_sequence = self.world.latest_room_event_sequence(&room).await?;
                Ok(WorldResponse {
                    ok: true,
                    room,
                    message: String::new(),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence,
                    broadcasted: true,
                    events: Vec::new(),
                    handle,
                    room_description: String::new(),
                    room_title: String::new(),
                    room_did: String::new(),
                    avatars: Vec::new(),
                    room_object_dids: HashMap::new(),
                    transport_ack: None,
                })
            }

            WorldRequest::Whisper { .. } => {
                Err(anyhow!("whisper requests are not supported on this lane"))
            }
        }
    }

    async fn handle_command(
        &self,
        command: WorldCommand,
        message: &Message,
        sender_did: &Did,
        sender_document: Document,
        agent_endpoint: String,
        sender_root: &str,
    ) -> Result<WorldResponse> {
        let sender_profile = sender_profile_from_document(&sender_document);

        match command {
            WorldCommand::Enter { room, preferred_handle } => {
                if self.lane != WorldLane::World {
                    return Err(anyhow!("enter is only supported on ma/world/1"));
                }
                let room = room.unwrap_or_else(|| DEFAULT_ROOM.to_string());
                let root_did = sender_did.without_fragment();
                let root_did_id = root_did.id();
                let is_first_enter = self
                    .world
                    .did_to_handle
                    .read()
                    .await
                    .get(&root_did_id)
                    .is_none();
                let closet_profile = self
                    .world
                    .closet_profiles
                    .read()
                    .await
                    .get(&root_did_id)
                    .cloned();
                let has_closet_profile = closet_profile
                    .as_ref()
                    .map(|profile| {
                        profile
                            .name
                            .as_ref()
                            .map(|v| !v.trim().is_empty())
                            .unwrap_or(false)
                            && profile
                                .description
                                .as_ref()
                                .map(|v| !v.trim().is_empty())
                                .unwrap_or(false)
                    })
                    .unwrap_or(false);

                if is_first_enter {
                    if !has_closet_profile {
                        return Ok(WorldResponse {
                            ok: false,
                            room,
                            message: "no avatar profile available yet; complete closet onboarding (name + description), then enter again".to_string(),
                            endpoint_id: self.endpoint_id.clone(),
                            latest_event_sequence: 0,
                            broadcasted: false,
                            events: Vec::new(),
                            handle: String::new(),
                            room_description: String::new(),
                            room_title: String::new(),
                            room_did: String::new(),
                            avatars: Vec::new(),
                            room_object_dids: HashMap::new(),
                            transport_ack: Some(TransportAck {
                                lane: self.lane.label().to_string(),
                                code: TransportAckCode::Rejected,
                                detail: "closet profile required".to_string(),
                            }),
                        });
                    }
                }

                if !self.world.can_enter(&root_did).await {
                    let (allow_all, allow_owner, allow_count, owner_did, acl_source) =
                        self.world.entry_acl_debug().await;
                    let (knock_id, duplicate) = self
                        .world
                        .enqueue_knock(
                            &room,
                            &root_did.id(),
                            &agent_endpoint,
                            preferred_handle.clone(),
                        )
                        .await;
                    let detail = if duplicate {
                        format!(
                            "entry denied by ACL for {}; existing knock request id={} is pending (acl_source='{}' allow_all={} allow_owner={} allow_count={} owner_did={})",
                            root_did.id(),
                            knock_id,
                            acl_source,
                            allow_all,
                            allow_owner,
                            allow_count,
                            owner_did.unwrap_or_else(|| "(none)".to_string())
                        )
                    } else {
                        format!(
                            "entry denied by ACL for {}; knock request queued with id={} (acl_source='{}' allow_all={} allow_owner={} allow_count={} owner_did={})",
                            root_did.id(),
                            knock_id,
                            acl_source,
                            allow_all,
                            allow_owner,
                            allow_count,
                            owner_did.unwrap_or_else(|| "(none)".to_string())
                        )
                    };
                    return Ok(WorldResponse {
                        ok: false,
                        room,
                        message: detail.clone(),
                        endpoint_id: self.endpoint_id.clone(),
                        latest_event_sequence: 0,
                        broadcasted: false,
                        events: Vec::new(),
                        handle: String::new(),
                        room_description: String::new(),
                        room_title: String::new(),
                        room_did: String::new(),
                        avatars: Vec::new(),
                        room_object_dids: HashMap::new(),
                        transport_ack: Some(TransportAck {
                            lane: self.lane.label().to_string(),
                            code: TransportAckCode::Rejected,
                            detail,
                        }),
                    });
                }
                let inbox = resolve_avatar_inbox(sender_did)?;
                let profile_name = closet_profile
                    .as_ref()
                    .and_then(|profile| profile.name.clone());
                let selected_handle = preferred_handle.or(profile_name);
                let Some(collapsed_language_order) = collapse_world_language_order_strict(&sender_profile) else {
                    return Ok(WorldResponse {
                        ok: false,
                        room,
                        message: format!(
                            "no supported language found in ma.language='{}'. supported={}. Choose language in closet, or leave.",
                            sender_profile,
                            supported_world_languages_text()
                        ),
                        endpoint_id: self.endpoint_id.clone(),
                        latest_event_sequence: 0,
                        broadcasted: false,
                        events: Vec::new(),
                        handle: String::new(),
                        room_description: String::new(),
                        room_title: String::new(),
                        room_did: String::new(),
                        avatars: Vec::new(),
                        room_object_dids: HashMap::new(),
                        transport_ack: Some(TransportAck {
                            lane: self.lane.label().to_string(),
                            code: TransportAckCode::Rejected,
                            detail: "language selection required".to_string(),
                        }),
                    });
                };
                let avatar_req = AvatarRequest {
                    inbox,
                    did: root_did,
                    agent_endpoint: agent_endpoint.clone(),
                    language_order: collapsed_language_order,
                };
                let handle = self.world.join_room(&room, avatar_req, selected_handle).await?;
                if let Some(profile) = closet_profile {
                    if let Some(description) = profile.description {
                        let _ = self
                            .world
                            .set_avatar_description_for_did(&room, &root_did_id, &description)
                            .await;
                    }
                }
                if room == DEFAULT_ROOM && is_first_enter && has_closet_profile {
                    let closet_exit_text = format!("{} comes out of the closet and into the lobby.", handle);
                    self.world
                        .record_room_event(
                            DEFAULT_ROOM,
                            "broadcast",
                            Some(handle.clone()),
                            Some(root_did_id.clone()),
                            Some(agent_endpoint.clone()),
                            closet_exit_text.clone(),
                        )
                        .await;
                    self.push_room_broadcast(DEFAULT_ROOM, Some(handle.clone()), closet_exit_text)
                        .await;
                }
                self.push_presence_snapshot(&room).await;
                let latest_event_sequence = self.world.latest_room_event_sequence(&room).await?;
                Ok(WorldResponse {
                    ok: true,
                    room: room.clone(),
                    message: format!("entered {room}"),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence,
                    broadcasted: false,
                    events: Vec::new(),
                    handle,
                    room_description: self.world.room_description(&room).await,
                    room_title: self.world.room_title(&room).await,
                    room_did: self.world.room_did(&room).await,
                    avatars: self.world.room_avatars(&room).await,
                    room_object_dids: self.world.room_object_did_map(&room).await,
                    transport_ack: None,
                })
            }
            WorldCommand::Message { room, envelope } => {
                let effective_sender_profile = self
                    .world
                    .avatar_language_order_for_did(&room, sender_root)
                    .await
                    .unwrap_or_else(|| "nb_NO:en_UK".to_string());
                let is_world_admin = matches!(
                    &envelope,
                    MessageEnvelope::ActorCommand { target, .. } if target.eq_ignore_ascii_case("world")
                );
                if is_world_admin && !self.world.is_world_target_did(&message.to).await {
                    return Err(anyhow!(
                        "@@ world commands must target this world DID; got to='{}'",
                        message.to
                    ));
                }
                match self.lane {
                    WorldLane::World if !is_world_admin => {
                        return Err(anyhow!("ma/world/1 accepts only @@ world commands for message requests"));
                    }
                    WorldLane::Cmd if is_world_admin => {
                        return Err(anyhow!("@@ world commands must be sent over ma/world/1"));
                    }
                    WorldLane::Chat => {
                        return Err(anyhow!("command envelopes are not supported on ma/chat/1"));
                    }
                    _ => {}
                }

                // Route command envelopes through the registered world handle for this DID root.
                // This keeps re-entry transparent when DID fragment/alias does not exactly match
                // the current in-room handle string.
                let actor_name = self.world.resolve_handle(sender_root).await;
                if actor_name == sender_root {
                    return Err(anyhow!("not registered in room — enter first"));
                }
                let (message, broadcasted, effective_room) = self
                    .world
                    .send_message(&room, &actor_name, sender_did, &effective_sender_profile, envelope)
                    .await?;
                if effective_room != room {
                    self.push_presence_snapshot(&room).await;
                    self.push_presence_snapshot(&effective_room).await;
                }
                let latest_event_sequence = self.world.latest_room_event_sequence(&effective_room).await?;
                Ok(WorldResponse {
                    ok: true,
                    room_description: self.world.room_description(&effective_room).await,
                    room_title: self.world.room_title(&effective_room).await,
                    room_did: self.world.room_did(&effective_room).await,
                    avatars: self.world.room_avatars(&effective_room).await,
                    room: effective_room.clone(),
                    message,
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence,
                    broadcasted,
                    events: Vec::new(),
                    handle: String::new(),
                    room_object_dids: self.world.room_object_did_map(&effective_room).await,
                    transport_ack: None,
                })
            }
            WorldCommand::RoomEvents { room, since_sequence } => {
                if self.lane != WorldLane::World {
                    return Err(anyhow!("room event polling is only supported on ma/world/1"));
                }
                let (events, latest_event_sequence) = self.world.room_events_since(&room, since_sequence).await?;
                Ok(WorldResponse {
                    ok: true,
                    room: room.clone(),
                    message: String::new(),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence,
                    broadcasted: false,
                    events,
                    handle: String::new(),
                    room_description: self.world.room_description(&room).await,
                    room_title: self.world.room_title(&room).await,
                    room_did: self.world.room_did(&room).await,
                    avatars: self.world.room_avatars(&room).await,
                    room_object_dids: self.world.room_object_did_map(&room).await,
                    transport_ack: None,
                })
            }
        }
    }

}

impl ProtocolHandler for WorldProtocol {
    /// One task runs per connection and serves a single long-lived bi-stream with framed messages.
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let agent_endpoint = connection.remote_id().to_string();
        self.world
            .record_event(format!("connection accepted from {}", agent_endpoint))
            .await;
        let (mut send, mut recv) = connection.accept_bi().await?;

        loop {
            let frame_len = match recv.read_u32().await {
                Ok(n) => n as usize,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(AcceptError::from_err(err)),
            };
            if frame_len > 256 * 1024 {
                return Err(AcceptError::from_err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("request frame too large: {}", frame_len),
                )));
            }

            let mut bytes = vec![0u8; frame_len];
            recv.read_exact(&mut bytes).await.map_err(AcceptError::from_err)?;

            let response = match serde_json::from_slice::<WorldRequest>(&bytes) {
                Ok(request) => self.process_request(request, agent_endpoint.clone()).await,
                Err(err) => WorldResponse {
                    ok: false,
                    room: String::new(),
                    message: format!("invalid request JSON: {}", err),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence: 0,
                    broadcasted: false,
                    events: Vec::new(),
                    handle: String::new(),
                    room_description: String::new(),
                    room_title: String::new(),
                    room_did: String::new(),
                    avatars: Vec::new(),
                    room_object_dids: HashMap::new(),
                    transport_ack: Some(TransportAck {
                        lane: self.lane.label().to_string(),
                        code: TransportAckCode::InvalidRequestJson,
                        detail: format!("invalid request JSON: {}", err),
                    }),
                },
            };
            let payload = serde_json::to_vec(&response).map_err(AcceptError::from_err)?;

            send.write_u32(payload.len() as u32)
                .await
                .map_err(AcceptError::from_err)?;
            send.write_all(&payload).await.map_err(AcceptError::from_err)?;
            send.flush().await.map_err(AcceptError::from_err)?;
        }

        let _ = send.finish();
        Ok(())
    }
}

impl ProtocolHandler for ClosetProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let requester_endpoint = connection.remote_id().to_string();
        let (mut send, mut recv) = connection.accept_bi().await?;

        loop {
            let frame_len = match recv.read_u32().await {
                Ok(n) => n as usize,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(AcceptError::from_err(err)),
            };

            if frame_len > 128 * 1024 {
                return Err(AcceptError::from_err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("closet frame too large: {}", frame_len),
                )));
            }

            let mut bytes = vec![0u8; frame_len];
            recv.read_exact(&mut bytes).await.map_err(AcceptError::from_err)?;

            let response = match serde_json::from_slice::<ClosetRequest>(&bytes) {
                Ok(ClosetRequest::Start) => {
                    match self.world.closet_start_session(&requester_endpoint).await {
                        Ok((session_id, latest_lobby_sequence)) => {
                            let active_lang = self
                                .world
                                .closet_lang_for_session(&session_id, &requester_endpoint)
                                .await;
                            ClosetResponse::ok(
                                &session_id,
                                tr_world(
                                    active_lang,
                                    "closet.session.ready",
                                    "closet session ready",
                                ),
                            )
                            .with_prompt(tr_world(
                                active_lang,
                                "closet.session.ready.prompt",
                                "Answer profile questions while waiting; you can hear lobby events from here.",
                            ))
                            .with_latest_lobby_sequence(latest_lobby_sequence)
                        }
                        Err(err) => ClosetResponse::err_unscoped(err.to_string()),
                    }
                }
                Ok(ClosetRequest::HearLobby {
                    session_id,
                    since_sequence,
                }) => {
                    match self
                        .world
                        .closet_session_owned_by(&session_id, &requester_endpoint)
                        .await
                    {
                        Ok(session) => match self.world.room_events_since(DEFAULT_ROOM, since_sequence).await {
                            Ok((events, latest_lobby_sequence)) => {
                                let active_lang = self
                                    .world
                                    .closet_lang_for_session(&session.id, &requester_endpoint)
                                    .await;
                                ClosetResponse::ok(
                                    &session.id,
                                    tr_world_vars(
                                        active_lang,
                                        "closet.session.active_since",
                                        &[("created_at", session.created_at.clone())],
                                        &format!("closet session active since {}", session.created_at),
                                    ),
                                )
                                .with_lobby_events(events, latest_lobby_sequence)
                            }
                            Err(err) => ClosetResponse::err_unscoped(err.to_string())
                                .with_latest_lobby_sequence(since_sequence),
                        },
                        Err(err) => ClosetResponse::err_unscoped(err.to_string())
                            .with_latest_lobby_sequence(since_sequence),
                    }
                }
                Ok(ClosetRequest::Answer {
                    session_id,
                    field,
                    value,
                }) => {
                    match self
                        .world
                        .closet_answer(&session_id, &requester_endpoint, &field, &value)
                        .await
                    {
                        Ok(message) => ClosetResponse::ok(&session_id, message),
                        Err(err) => ClosetResponse::err_unscoped(err.to_string()),
                    }
                }
                Ok(ClosetRequest::SubmitCitizenship {
                    session_id,
                    ipns_private_key_base64,
                    desired_fragment,
                }) => {
                    let desired_fragment = desired_fragment
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    match self
                        .world
                        .closet_submit_citizenship(
                            &session_id,
                            &requester_endpoint,
                            &ipns_private_key_base64,
                            desired_fragment,
                        )
                        .await
                    {
                        Ok((did, fragment, key_name)) => {
                            let active_lang = self
                                .world
                                .closet_lang_for_session(&session_id, &requester_endpoint)
                                .await;
                            ClosetResponse::ok(
                                &session_id,
                                tr_world(
                                    active_lang,
                                    "closet.citizenship.imported",
                                    "citizenship imported",
                                ),
                            )
                            .with_prompt(tr_world(
                                active_lang,
                                "closet.citizenship.prompt",
                                "Citizenship granted. Rebind your local identity to the returned DID and enter the world.",
                            ))
                            .with_did(did)
                            .with_fragment(fragment)
                            .with_key_name(key_name)
                        }
                        Err(err) => ClosetResponse::err_unscoped(err.to_string()),
                    }
                }
                Ok(ClosetRequest::PublishDidDocument {
                    session_id,
                    did_document_json,
                    ipns_private_key_base64,
                    desired_fragment,
                }) => {
                    let desired_fragment = desired_fragment
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    match self
                        .world
                        .closet_publish_did_document(
                            &session_id,
                            &requester_endpoint,
                            &did_document_json,
                            &ipns_private_key_base64,
                            desired_fragment,
                        )
                        .await
                    {
                        Ok((did, cid, key_name)) => {
                            let active_lang = self
                                .world
                                .closet_lang_for_session(&session_id, &requester_endpoint)
                                .await;
                            ClosetResponse::ok(
                                &session_id,
                                tr_world_vars(
                                    active_lang,
                                    "closet.did.published",
                                    &[("cid", cid.clone())],
                                    &format!("did document published at /ipfs/{}", cid),
                                ),
                            )
                            .with_did(did)
                            .with_fragment(key_name.clone())
                            .with_key_name(key_name)
                        }
                        Err(err) => ClosetResponse::err_unscoped(err.to_string()),
                    }
                }
                Ok(ClosetRequest::Command { session_id, input }) => {
                    match self
                        .world
                        .closet_command(&session_id, &requester_endpoint, &input)
                        .await
                    {
                        Ok(response) => response,
                        Err(err) => ClosetResponse::err_unscoped(err.to_string()),
                    }
                }
                Err(err) => ClosetResponse::err_unscoped(format!(
                    "invalid closet request JSON: {}",
                    err
                )),
            };

            let payload = serde_json::to_vec(&response).map_err(AcceptError::from_err)?;
            send.write_u32(payload.len() as u32)
                .await
                .map_err(AcceptError::from_err)?;
            send.write_all(&payload).await.map_err(AcceptError::from_err)?;
            send.flush().await.map_err(AcceptError::from_err)?;
        }

        let _ = send.finish();
        Ok(())
    }
}


fn derive_world_master_key(secret_key: &SecretKey, world_slug: &str) -> [u8; 32] {
    // Deterministic per-world key derived from machine-local iroh identity.
    let mut hasher = Sha256::new();
    hasher.update(b"ma-world/master-key/v1");
    hasher.update(world_slug.as_bytes());
    hasher.update(secret_key.to_bytes());
    let digest = hasher.finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn derive_world_signing_private_key(world_master_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"ma-world/did-doc-signing-key/v1");
    hasher.update(world_master_key);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn derive_world_encryption_private_key(world_master_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"ma-world/did-doc-encryption-key/v1");
    hasher.update(world_master_key);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn runtime_state_key_name(world_slug: &str) -> String {
    format!("{}-state", normalize_world_key_name(world_slug))
}

fn ma_pointer_mode_enabled() -> bool {
    match std::env::var("MA_WORLD_MA_POINTER") {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "yes" || normalized == "on"
        }
        Err(_) => false,
    }
}

async fn ensure_kubo_key_id(kubo_url: &str, key_name: &str) -> Result<String> {
    let mut keys = list_kubo_keys(kubo_url).await?;
    if !keys.iter().any(|key| key.name == key_name) {
        generate_kubo_key(kubo_url, key_name).await?;
        keys = list_kubo_keys(kubo_url).await?;
    }

    keys
        .into_iter()
        .find(|key| key.name == key_name)
        .map(|key| key.id)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| anyhow!("kubo key '{}' exists but has no usable id", key_name))
}

async fn resolve_world_pointer_from_did(kubo_url: &str, world_did: &str) -> Result<(Option<String>, Option<String>)> {
    let world = Did::try_from(world_did)
        .map_err(|e| anyhow!("invalid world DID '{}': {}", world_did, e))?
        .without_fragment();
    let document = kubo::fetch_did_document(kubo_url, &world).await?;
    let Some(ma) = document.ma.as_ref() else {
        return Ok((None, None));
    };
    let Some(link) = ma.link.as_ref() else {
        return Ok((None, None));
    };

    let target = link.trim();
    if target.is_empty() {
        return Ok((None, None));
    }

    if let Some(cid) = target.strip_prefix("/ipfs/") {
        let value = cid.trim();
        return if value.is_empty() {
            Ok((Some(target.to_string()), None))
        } else {
            Ok((Some(target.to_string()), Some(value.to_string())))
        };
    }

    if target.starts_with("/ipns/") {
        let resolved = name_resolve(kubo_url, target, true).await?;
        let Some(cid) = resolved.strip_prefix("/ipfs/") else {
            return Err(anyhow!("ma link '{}' resolved to non-/ipfs path '{}'", target, resolved));
        };
        let value = cid.trim();
        return if value.is_empty() {
            Ok((Some(target.to_string()), None))
        } else {
            Ok((Some(target.to_string()), Some(value.to_string())))
        };
    }

    Ok((Some(target.to_string()), Some(target.to_string())))
}

async fn resolve_world_root_cid_from_did_pointer(kubo_url: &str, world_did: &str) -> Result<Option<String>> {
    let (_, root_cid) = resolve_world_pointer_from_did(kubo_url, world_did).await?;
    Ok(root_cid)
}

async fn resolve_world_root_cid_from_did_inline(kubo_url: &str, world_did: &str) -> Result<Option<String>> {
    let world = Did::try_from(world_did)
        .map_err(|e| anyhow!("invalid world DID '{}': {}", world_did, e))?
        .without_fragment();
    let document = kubo::fetch_did_document(kubo_url, &world).await?;
    let Some(ma) = document.ma.as_ref() else {
        return Ok(None);
    };
    let Some(root_cid) = ma.world_root_cid.as_ref() else {
        return Ok(None);
    };
    let value = root_cid.trim();
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value.to_string()))
}

async fn publish_world_did_runtime_ma(
    kubo_url: &str,
    world_slug: &str,
    world_master_key: [u8; 32],
    state_cid: &str,
    root_cid: &str,
    pointer_mode: bool,
) -> Result<()> {
    let world_key_name = normalize_world_key_name(world_slug);
    let did_identifier = ensure_kubo_key_id(kubo_url, &world_key_name).await?;
    let world_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;

    let signing_did = Did::new(&did_identifier, "sig")?;
    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        derive_world_signing_private_key(&world_master_key),
    )
    .map_err(|e| anyhow!("failed to restore world signing key: {}", e))?;

    let mut document = kubo::fetch_did_document(kubo_url, &world_did).await?;
    document.set_ma_type("world");

    if pointer_mode {
        let state_key_name = runtime_state_key_name(world_slug);
        let state_ipns_id = ensure_kubo_key_id(kubo_url, &state_key_name).await?;
        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_secs(45),
            ..IpnsPublishOptions::default()
        };
        ipns_publish_with_retry(
            kubo_url,
            &state_key_name,
            root_cid,
            &ipns_options,
            8,
            Duration::from_millis(1500),
        )
        .await?;

        document.set_ma_link(format!("/ipns/{}", state_ipns_id));
        document.clear_ma_state_cid();
        document.clear_ma_world_root_cid();
    } else {
        document.clear_ma_link();
        document.set_ma_state_cid(state_cid);
        document.set_ma_world_root_cid(root_cid);
    }

    let assertion_vm = document
        .get_verification_method_by_id(&document.assertion_method)
        .map_err(|e| anyhow!("world DID missing assertion method '{}': {}", document.assertion_method, e))?
        .clone();
    document.sign(&signing_key, &assertion_vm)?;

    let document_json = document
        .marshal()
        .map_err(|e| anyhow!("failed to marshal world DID document: {}", e))?;
    let document_cid = ipfs_add(kubo_url, document_json.into_bytes()).await?;

    let ipns_options = IpnsPublishOptions {
        timeout: Duration::from_secs(45),
        ..IpnsPublishOptions::default()
    };
    ipns_publish_with_retry(
        kubo_url,
        &world_key_name,
        &document_cid,
        &ipns_options,
        8,
        Duration::from_millis(1500),
    )
    .await?;

    Ok(())
}

async fn ensure_world_did_document(
    kubo_url: &str,
    world_slug: &str,
    endpoint_id: &str,
    world_master_key: [u8; 32],
    pointer_mode: bool,
) -> Result<String> {
    let key_name = normalize_world_key_name(world_slug);
    let mut keys = list_kubo_keys(kubo_url).await?;
    if !keys.iter().any(|key| key.name == key_name) {
        generate_kubo_key(kubo_url, &key_name).await?;
        keys = list_kubo_keys(kubo_url).await?;
    }

    let did_identifier = keys
        .iter()
        .find(|key| key.name == key_name)
        .map(|key| key.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!(
            "kubo key '{}' exists but did not expose a usable Id in key/list",
            key_name
        ))?;

    let world_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build world DID from IPNS key Id '{}': {}", did_identifier, e))?;

    let state_ipns_id = if pointer_mode {
        let state_key_name = runtime_state_key_name(world_slug);
        Some(ensure_kubo_key_id(kubo_url, &state_key_name).await?)
    } else {
        None
    };

    let signing_did = Did::new(&did_identifier, "sig")?;
    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        derive_world_signing_private_key(&world_master_key),
    )
        .map_err(|e| anyhow!("failed to restore world signing key: {}", e))?;

    let key_agreement_did = Did::new(&did_identifier, "enc")?;
    let key_agreement_key = EncryptionKey::from_private_key_bytes(
        key_agreement_did,
        derive_world_encryption_private_key(&world_master_key),
    )
    .map_err(|e| anyhow!("failed to restore world key-agreement key: {}", e))?;

    let mut document = Document::new(&world_did, &world_did);

    let assertion_vm = VerificationMethod::new(
        world_did.base_id(),
        world_did.base_id(),
        signing_key.key_type.clone(),
        "sig",
        signing_key.public_key_multibase.clone(),
    )
    .map_err(|e| anyhow!("failed building world assertion method: {}", e))?;

    let key_agreement_vm = VerificationMethod::new(
        world_did.base_id(),
        world_did.base_id(),
        key_agreement_key.key_type.clone(),
        "enc",
        key_agreement_key.public_key_multibase.clone(),
    )
    .map_err(|e| anyhow!("failed building world keyAgreement method: {}", e))?;

    let assertion_vm_id = assertion_vm.id.clone();
    let key_agreement_vm_id = key_agreement_vm.id.clone();
    document.add_verification_method(assertion_vm.clone())?;
    document.add_verification_method(key_agreement_vm)?;
    document.assertion_method = assertion_vm_id;
    document.key_agreement = key_agreement_vm_id;
    document.set_ma_type("world");
    if let Some(state_ipns_id) = state_ipns_id {
        document.set_ma_link(format!("/ipns/{}", state_ipns_id));
    } else {
        document.clear_ma_link();
    }
    let transport_paths = vec![
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(WORLD_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(CMD_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(CHAT_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(CLOSET_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(PRESENCE_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(BROADCAST_ALPN)),
    ];
    document.set_ma_transports(serde_json::Value::Array(
        transport_paths
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    ));
    document.sign(&signing_key, &assertion_vm)?;

    let document_json = document
        .marshal()
        .map_err(|e| anyhow!("failed to marshal world DID document: {}", e))?;
    let document_cid = ipfs_add(kubo_url, document_json.into_bytes()).await?;

    let ipns_options = IpnsPublishOptions {
        timeout: Duration::from_secs(45),
        ..IpnsPublishOptions::default()
    };

    let published = ipns_publish_with_retry(
        kubo_url,
        &key_name,
        &document_cid,
        &ipns_options,
        8,
        Duration::from_millis(1500),
    )
    .await?;

    info!(
        "Upserted world DID document {} as CID {} (IPNS {})",
        world_did.id(),
        document_cid,
        published
    );

    Ok(world_did.id())
}

#[tokio::main]
async fn main() -> Result<()> {
    let raw_args = std::env::args().collect::<Vec<_>>();
    let args = extract_global_config_arg(raw_args)?;
    let mut run_arg_mode = false;
    let mut listen_addr: String = DEFAULT_LISTEN_ADDR.to_string();
    let mut kubo_url_override: Option<String> = None;
    let mut log_level: String = "info".to_string();
    let mut log_file: Option<PathBuf> = None;
    let mut world_slug_override: Option<String> = None;
    let mut actor_web_cid_override: Option<String> = None;

    if args.len() >= 2 {
        match args[1].as_str() {
            "-h" | "--help" | "help" => {
                print_cli_help();
                return Ok(());
            }
            "--gen-iroh-secret" => {
                let mut explicit_path: Option<PathBuf> = None;
                let mut world_slug = DEFAULT_WORLD_SLUG.to_string();
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--world-slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --world-slug"));
                            }
                            world_slug = args[idx].clone();
                        }
                        other => {
                            if explicit_path.is_some() {
                                return Err(anyhow!(
                                    "usage: ma-world --gen-iroh-secret [path] [--world-slug <slug>]"
                                ));
                            }
                            explicit_path = Some(PathBuf::from(other));
                        }
                    }
                    idx += 1;
                }

                let normalized_slug = normalize_world_key_name(&world_slug);
                let runtime_cfg_path = runtime_config_path(&normalized_slug);
                let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
                let path = explicit_path
                    .or_else(|| runtime_cfg.iroh_secret.map(PathBuf::from))
                    .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));

                generate_iroh_secret_file(&path)?;
                println!("generated iroh secret: {}", path.display());
                return Ok(());
            }
            "run" => {
                run_arg_mode = true;
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--listen" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --listen"));
                            }
                            listen_addr = args[idx].clone();
                        }
                        "--kubo-url" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --kubo-url"));
                            }
                            kubo_url_override = Some(args[idx].clone());
                        }
                        "--cid" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --cid"));
                            }
                            actor_web_cid_override = Some(args[idx].clone());
                        }
                        "--log-level" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --log-level"));
                            }
                            log_level = args[idx].clone();
                        }
                        "--log-file" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --log-file"));
                            }
                            log_file = Some(PathBuf::from(&args[idx]));
                        }
                        "--world-slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --world-slug"));
                            }
                            world_slug_override = Some(args[idx].clone());
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for run (supported: --world-slug, --listen, --kubo-url, --cid, --log-level, --log-file)",
                                other
                            ));
                        }
                    }
                    idx += 1;
                }
            }
            _ => {}
        }
    }

    // Backwards-compatible server mode with top-level flags and no explicit command.
    if !run_arg_mode && args.len() >= 2 && args[1].starts_with('-') {
        run_arg_mode = true;
        let mut idx = 1usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--listen" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --listen"));
                    }
                    listen_addr = args[idx].clone();
                }
                "--kubo-url" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --kubo-url"));
                    }
                    kubo_url_override = Some(args[idx].clone());
                }
                "--cid" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --cid"));
                    }
                    actor_web_cid_override = Some(args[idx].clone());
                }
                "--log-level" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --log-level"));
                    }
                    log_level = args[idx].clone();
                }
                "--log-file" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --log-file"));
                    }
                    log_file = Some(PathBuf::from(&args[idx]));
                }
                "--world-slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-slug"));
                    }
                    world_slug_override = Some(args[idx].clone());
                }
                other => {
                    return Err(anyhow!(
                        "unknown top-level argument '{}'. Use 'publish-world' for publish flags like --skip-ipns, or use run/--world-slug/--listen/--kubo-url/--cid for server mode.",
                        other
                    ));
                }
            }
            idx += 1;
        }
    }

    let default_world_slug = DEFAULT_WORLD_SLUG.to_string();
    let default_kubo_url = DEFAULT_KUBO_API_URL.to_string();

    let runtime_slug = if run_arg_mode {
        Some(
            world_slug_override
                .clone()
                .ok_or_else(|| anyhow!("--world-slug is required for server mode"))?,
        )
    } else {
        None
    };

    // Apply runtime values with precedence: CLI args > config file > env vars > defaults.
    if let Some(slug) = runtime_slug.as_deref() {
        let normalized_slug = normalize_world_key_name(slug);
        let runtime_config_path = runtime_config_path(&normalized_slug);
        let runtime_file_config = load_runtime_file_config(&runtime_config_path)?;

        if listen_addr == DEFAULT_LISTEN_ADDR {
            if let Some(cfg_listen) = runtime_file_config.listen.clone() {
                listen_addr = cfg_listen;
            } else if let Ok(env_listen) = std::env::var("MA_LISTEN") {
                listen_addr = env_listen;
            }
        }

        if kubo_url_override.is_none() {
            if let Some(cfg_kubo) = runtime_file_config.kubo_api_url.clone() {
                kubo_url_override = Some(cfg_kubo);
            } else if let Ok(env_kubo) = std::env::var("MA_KUBO_API_URL") {
                kubo_url_override = Some(env_kubo);
            }
        }

        if log_level == "info" {
            if let Some(cfg_level) = runtime_file_config.log_level.clone() {
                log_level = cfg_level;
            } else if let Ok(env_level) = std::env::var("MA_LOG_LEVEL") {
                log_level = env_level;
            }
        }

        if log_file.is_none() {
            if let Some(cfg_file) = runtime_file_config.log_file.clone() {
                log_file = Some(PathBuf::from(cfg_file));
            } else if let Ok(env_file) = std::env::var("MA_LOG_FILE") {
                log_file = Some(PathBuf::from(env_file));
            }
        }
    }

    if args.len() >= 2 && args[1] == "check-kubo-ipns" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;
        let mut key_override: Option<String> = None;
        let mut ipns_timeout_ms: u64 = 15_000;
        let mut ipns_retries: u32 = 3;
        let mut ipns_backoff_ms: u64 = 1_000;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--world-slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                "--key" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --key"));
                    }
                    key_override = Some(args[idx].clone());
                }
                "--ipns-timeout-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-timeout-ms"));
                    }
                    ipns_timeout_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-timeout-ms: {}", e))?;
                }
                "--ipns-retries" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-retries"));
                    }
                    ipns_retries = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-retries: {}", e))?;
                }
                "--ipns-backoff-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-backoff-ms"));
                    }
                    ipns_backoff_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-backoff-ms: {}", e))?;
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for check-kubo-ipns (supported: --world-slug, --world-dir, --key, --ipns-timeout-ms, --ipns-retries, --ipns-backoff-ms)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let loaded = load_world_authoring(&world_dir)?;
        let kubo_url = loaded.config.kubo.api_url.clone();
        let key_name = key_override.unwrap_or_else(|| loaded.config.kubo.world_ipns_key_name.clone());

        let available = list_kubo_key_names(&kubo_url).await?;
        if !available.iter().any(|candidate| candidate == &key_name) {
            return Err(anyhow!(
                "Kubo key '{}' is missing (run ensure-kubo-keys first)",
                key_name
            ));
        }

        let probe = serde_json::json!({
            "kind": "ma.kubo-healthcheck.v1",
            "timestamp": Utc::now().to_rfc3339(),
            "world": loaded.world_manifest.world_id,
        });
        let probe_cid = dag_put_dag_cbor(&kubo_url, &probe).await?;

        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_millis(ipns_timeout_ms),
            ..IpnsPublishOptions::default()
        };
        let published = ipns_publish_with_retry(
            &kubo_url,
            &key_name,
            &probe_cid,
            &ipns_options,
            ipns_retries,
            Duration::from_millis(ipns_backoff_ms),
        )
        .await?;

        println!("check-kubo-ipns OK");
        println!("  key: {}", key_name);
        println!("  probe_cid: {}", probe_cid);
        println!("  published: {}", published);
        println!("  retries: {}", ipns_retries);
        println!("  timeout_ms: {}", ipns_timeout_ms);
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "init-world" {
        return Err(anyhow!("init-world has been removed"));
    }

    if args.len() >= 2 && args[1] == "ensure-kubo-keys" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--world-slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for ensure-kubo-keys (supported: --world-slug, --world-dir)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let loaded = load_world_authoring(&world_dir)?;
        let kubo_url = loaded.config.kubo.api_url.clone();
        let mut available = list_kubo_key_names(&kubo_url).await?;

        let mut expected = loaded.config.kubo.required_keys.clone();
        expected.push(loaded.config.kubo.world_ipns_key_name.clone());
        for actor in loaded.actors.values() {
            if actor.actor_type != "room" {
                continue;
            }
            if let Some(fragment) = did_fragment(&actor.id) {
                expected.push(fragment.to_string());
            }
        }
        expected.sort();
        expected.dedup();

        let mut created = Vec::new();
        for key in &expected {
            if available.iter().any(|candidate| candidate == key) {
                continue;
            }
            generate_kubo_key(&kubo_url, key).await?;
            created.push(key.clone());
            available.push(key.clone());
        }

        println!("ensure-kubo-keys OK");
        if created.is_empty() {
            println!("  created: (none)");
        } else {
            println!("  created: {}", created.join(", "));
        }
        println!("  required: {}", expected.join(", "));
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "check-kubo-keys" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--world-slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for check-kubo-keys (supported: --world-slug, --world-dir)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let loaded = load_world_authoring(&world_dir)?;
        let available = list_kubo_key_names(&loaded.config.kubo.api_url).await?;

        let mut expected = loaded.config.kubo.required_keys.clone();
        expected.push(loaded.config.kubo.world_ipns_key_name.clone());
        for actor in loaded.actors.values() {
            if actor.actor_type != "room" {
                continue;
            }
            if let Some(fragment) = did_fragment(&actor.id) {
                expected.push(fragment.to_string());
            }
        }
        expected.sort();
        expected.dedup();

        let missing = expected
            .iter()
            .filter(|key| !available.iter().any(|candidate| candidate == *key))
            .cloned()
            .collect::<Vec<_>>();

        if !missing.is_empty() {
            return Err(anyhow!(
                "missing Kubo key(s): {}",
                missing.join(", ")
            ));
        }

        println!("kubo key check OK");
        println!("  required: {}", expected.join(", "));
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "publish-world" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;
        let mut skip_ipns = false;
        let mut allow_partial_ipns = false;
        let mut ipns_timeout_ms: u64 = 15_000;
        let mut ipns_retries: u32 = 3;
        let mut ipns_backoff_ms: u64 = 1_000;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--world-slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                "--skip-ipns" => {
                    skip_ipns = true;
                }
                "--allow-partial-ipns" => {
                    allow_partial_ipns = true;
                }
                "--ipns-timeout-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-timeout-ms"));
                    }
                    ipns_timeout_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-timeout-ms: {}", e))?;
                }
                "--ipns-retries" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-retries"));
                    }
                    ipns_retries = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-retries: {}", e))?;
                }
                "--ipns-backoff-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-backoff-ms"));
                    }
                    ipns_backoff_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-backoff-ms: {}", e))?;
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for publish-world (supported: --world-slug, --world-dir, --skip-ipns, --allow-partial-ipns, --ipns-timeout-ms, --ipns-retries, --ipns-backoff-ms)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let mut loaded = load_world_authoring(&world_dir)?;
        let kubo_url = loaded.config.kubo.api_url.clone();

        let mut actor_cids: HashMap<String, String> = HashMap::new();
        let actor_ids = loaded.actors.keys().cloned().collect::<Vec<_>>();
        for actor_id in actor_ids {
            let actor = loaded
                .actors
                .get(&actor_id)
                .ok_or_else(|| anyhow!("missing actor payload for {}", actor_id))?;
            let cid = dag_put_dag_cbor(&kubo_url, actor).await?;
            actor_cids.insert(actor_id, cid);
        }

        for (actor_id, item) in &mut loaded.actor_registry.actors {
            if let Some(cid) = actor_cids.get(actor_id) {
                item.cid = cid.clone();
            }
        }

        let actor_registry_cid = dag_put_dag_cbor(&kubo_url, &loaded.actor_registry).await?;
        loaded.world_root.refs.actor_registry_cid = actor_registry_cid.clone();
        let world_root_cid = dag_put_dag_cbor(&kubo_url, &loaded.world_root).await?;

        loaded.world_manifest.world_root_cid = world_root_cid.clone();
        loaded.world_manifest.refs.actor_registry_cid = actor_registry_cid.clone();
        let world_manifest_cid = dag_put_dag_cbor(&kubo_url, &loaded.world_manifest).await?;

        let mut ipns_failures: Vec<String> = Vec::new();
        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_millis(ipns_timeout_ms),
            ..IpnsPublishOptions::default()
        };

        if loaded.config.publish.publish_world_ipns && !skip_ipns {
            let available = list_kubo_key_names(&kubo_url).await?;
            let mut required = loaded.config.kubo.required_keys.clone();
            required.push(loaded.config.kubo.world_ipns_key_name.clone());
            for actor in loaded.actors.values() {
                if actor.actor_type != "room" {
                    continue;
                }
                if let Some(fragment) = did_fragment(&actor.id) {
                    required.push(fragment.to_string());
                }
            }
            required.sort();
            required.dedup();
            let missing = required
                .iter()
                .filter(|key| !available.iter().any(|candidate| candidate == *key))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                let missing_msg = format!("cannot publish IPNS: missing Kubo key(s): {}", missing.join(", "));
                if allow_partial_ipns {
                    ipns_failures.push(missing_msg);
                } else {
                    return Err(anyhow!(missing_msg));
                }
            }

            if ipns_failures.is_empty() {
                match ipns_publish_with_retry(
                    &kubo_url,
                    &loaded.config.kubo.world_ipns_key_name,
                    &world_manifest_cid,
                    &ipns_options,
                    ipns_retries,
                    Duration::from_millis(ipns_backoff_ms),
                )
                .await
                {
                    Ok(published) => println!("world manifest published to IPNS: {}", published),
                    Err(err) => {
                        let msg = format!(
                            "world manifest IPNS publish failed (key={}): {}",
                            loaded.config.kubo.world_ipns_key_name,
                            err
                        );
                        if allow_partial_ipns {
                            ipns_failures.push(msg);
                        } else {
                            return Err(anyhow!(msg));
                        }
                    }
                }

                for actor in loaded.actors.values() {
                    if actor.actor_type != "room" {
                        continue;
                    }
                    let room_key = did_fragment(&actor.id)
                        .ok_or_else(|| anyhow!("room actor id missing fragment: {}", actor.id))?;
                    let room_cid = actor_cids
                        .get(&actor.id)
                        .ok_or_else(|| anyhow!("missing actor cid for room {}", actor.id))?;
                    match ipns_publish_with_retry(
                        &kubo_url,
                        room_key,
                        room_cid,
                        &ipns_options,
                        ipns_retries,
                        Duration::from_millis(ipns_backoff_ms),
                    )
                    .await
                    {
                        Ok(room_published) => {
                            println!("room actor published to IPNS ({}): {}", room_key, room_published)
                        }
                        Err(err) => {
                            let msg = format!(
                                "room actor IPNS publish failed (key={}): {}",
                                room_key, err
                            );
                            if allow_partial_ipns {
                                ipns_failures.push(msg);
                            } else {
                                return Err(anyhow!(msg));
                            }
                        }
                    }
                }
            }
        } else {
            println!("skipped IPNS publish (publish_world_ipns=false or --skip-ipns)");
        }

        if ipns_failures.is_empty() {
            println!("publish-world OK");
        } else {
            println!("publish-world PARTIAL (IPFS/IPLD succeeded; IPNS had issues)");
            for failure in &ipns_failures {
                println!("  ipns_issue: {}", failure);
            }
        }
        println!("  world_dir: {}", loaded.world_dir.display());
        println!("  actor_registry_cid: {}", actor_registry_cid);
        println!("  world_root_cid: {}", world_root_cid);
        println!("  world_manifest_cid: {}", world_manifest_cid);
        if loaded.config.publish.publish_world_ipns && !skip_ipns {
            println!("  ipns_retries: {}", ipns_retries);
            println!("  ipns_timeout_ms: {}", ipns_timeout_ms);
            println!("  ipns_backoff_ms: {}", ipns_backoff_ms);
            println!("  allow_partial_ipns: {}", allow_partial_ipns);
        }
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "validate-world" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--world-slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for validate-world (supported: --world-slug, --world-dir)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let report = validate_world_authoring(&world_dir)?;
        println!("world schema validation OK");
        println!("  world_dir: {}", report.world_dir);
        println!("  config: {}", report.config_path);
        println!("  authoring: {}", report.authoring_dir);
        println!("  actors: {}", report.actor_count);
        return Ok(());
    }

    if args.len() >= 2 && !run_arg_mode {
        return Err(anyhow!(
            "unknown command '{}'. Use --help to list available commands.",
            args[1]
        ));
    }

    // Initialize logging with configurable level and optional file output
    let directive = format!("ma_world={}", log_level);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(directive.parse()?);

    if let Some(log_file_path) = &log_file {
        // Create parent directory if it doesn't exist
        if let Some(parent) = log_file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)?;

        let stdout_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(true)
            .with_writer(std::io::stdout);

        let file_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(false)
            .with_writer(file);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stdout_layer)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(true)
            .init();
    }

    info!("Starting ma-world server");
    info!("Log level: {}", log_level);
    if let Some(log_path) = &log_file {
        info!("Logging to file: {}", log_path.display());
    }

    let runtime_slug = runtime_slug
        .ok_or_else(|| anyhow!("--world-slug is required for server mode"))?;
    let world_slug = normalize_world_key_name(&runtime_slug);
    let runtime_cfg_path = runtime_config_path(&world_slug);
    let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
    let authored_world_dir = default_world_dir(&world_slug);
    let authored_world = load_world_authoring(&authored_world_dir).ok();
    let authored_global_acl_cid = authored_world
        .as_ref()
        .and_then(|loaded| loaded.world_root.refs.global_acl_cid.clone())
        .filter(|cid| !cid.trim().is_empty());
    let authored_actor_web = authored_world
        .and_then(|loaded| loaded.world_manifest.actor_web)
        .and_then(|registry| {
            registry
                .active_artifact()
                .map(|artifact| (artifact.version.clone(), artifact.cid.clone()))
        });

    let kubo_url = kubo_url_override
        .or_else(|| runtime_cfg.kubo_api_url.clone())
        .or_else(|| std::env::var("MA_KUBO_API_URL").ok())
        .unwrap_or_else(|| default_kubo_url.clone());
    let entry_acl = load_entry_acl()?;
    let world = Arc::new(World::new(
        entry_acl,
        kubo_url.clone(),
        world_slug.clone(),
    ));
    info!("Runtime world slug: {}", world_slug);
    info!("Runtime config path: {}", runtime_cfg_path.display());

    world.create_room(DEFAULT_ROOM.to_string()).await?;

    // Bind status listener before iroh endpoint setup so listen failures abort early.
    let listener = bind_status_listener(&listen_addr).await?;
    let status_addr = listener.local_addr()?;
    let status_url = format!("http://{}", status_addr);

    let actor_web_listen = runtime_cfg
        .actor_web_listen
        .clone()
        .unwrap_or_else(|| "127.0.0.1:8081".to_string());
    let actor_web_ipns_key = runtime_cfg
        .actor_web_ipns_key
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ma-actor".to_string());
    let runtime_actor_web_version = runtime_cfg
        .actor_web_version
        .clone()
        .filter(|value| !value.trim().is_empty());
    let runtime_actor_web_cid = runtime_cfg
        .actor_web_cid
        .clone()
        .filter(|value| !value.trim().is_empty());
    let actor_web_auto_build = runtime_cfg.actor_web_auto_build.unwrap_or(true);
    let actor_web_auto_publish_ipns = runtime_cfg.actor_web_auto_publish_ipns.unwrap_or(true);
    let manual_actor_web_cid = actor_web_cid_override
        .clone()
        .or(runtime_actor_web_cid);
    let authored_actor_web_version = authored_actor_web.as_ref().map(|(version, _)| version.clone());
    let authored_actor_web_cid = authored_actor_web.as_ref().map(|(_, cid)| cid.clone());
    let actor_web_version = runtime_actor_web_version
        .or(authored_actor_web_version)
        .or_else(|| Some("local-dev".to_string()));

    let mut actor_web_cid = manual_actor_web_cid;
    let mut actor_web_source_dir = resolve_actor_web_source_dir(&runtime_cfg);

    let key_path = runtime_cfg
        .iroh_secret
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_iroh_secret_default_path(&world_slug));
    let Some(secret_key) = load_persisted_iroh_secret_key(&key_path)? else {
        return Err(anyhow!(
            "missing iroh secret at {}. Create it explicitly with: ma-world --gen-iroh-secret {}",
            key_path.display(),
            key_path.display()
        ));
    };
    info!("Loaded persistent iroh identity from {}", key_path.display());
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .bind()
        .await?;

    // Ensure Kubo API is online before DID/IPNS bootstrap.
    wait_for_kubo_api(&kubo_url, 8, Duration::from_millis(1000)).await?;

    if let Some(global_acl_cid) = authored_global_acl_cid.as_deref() {
        match world.load_global_capability_acl_from_cid(global_acl_cid).await {
            Ok(()) => info!("Loaded global capability ACL from CID {}", global_acl_cid),
            Err(err) => warn!(
                "Failed loading global capability ACL from CID {}: {}",
                global_acl_cid,
                err
            ),
        }
    }

    if actor_web_cid.is_none() && actor_web_auto_build {
        if let Some(source_dir) = actor_web_source_dir.as_deref() {
            let maybe_ipns_key = if actor_web_auto_publish_ipns {
                Some(actor_web_ipns_key.as_str())
            } else {
                None
            };
            match publish_actor_web_from_dir(&kubo_url, source_dir, maybe_ipns_key).await {
                Ok(cid) => {
                    info!(
                        "Auto-built actor web CID {} from {}{}",
                        cid,
                        source_dir.display(),
                        if actor_web_auto_publish_ipns {
                            format!(" and published via key '{}'", actor_web_ipns_key)
                        } else {
                            String::new()
                        }
                    );
                    actor_web_cid = Some(cid);
                }
                Err(err) => {
                    warn!(
                        "Actor web auto-build failed from {}: {}",
                        source_dir.display(),
                        err
                    );
                }
            }
        } else {
            warn!(
                "Actor web auto-build enabled, but no source dir found (set actor_web_dir or keep sibling ma-actor/www)"
            );
        }
    }

    if actor_web_cid.is_none() {
        actor_web_cid = authored_actor_web_cid;
    }

    if actor_web_cid.is_none() {
        if let Some(cid) = resolve_actor_web_cid_from_ipns_key(&kubo_url, &actor_web_ipns_key).await? {
            info!(
                "Resolved actor web CID {} from Kubo key '{}'",
                cid,
                actor_web_ipns_key
            );
            actor_web_cid = Some(cid);
        }
    }

    if actor_web_source_dir.is_none() {
        if let Some(cid) = actor_web_cid.as_deref() {
            let cache_root = runtime_cfg
                .actor_web_cache_dir
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(|| xdg_data_home().join("ma").join("actor-web"));
            actor_web_source_dir = Some(materialize_actor_web_from_cid(&kubo_url, cid, &cache_root).await?);
        }
    }

    let actor_web_runtime = actor_web_source_dir.map(|source_dir| {
        (
            source_dir,
            actor_web_version.clone(),
            actor_web_cid.clone(),
        )
    });

    let run_result: Result<()> = async {
        let world_master_key = derive_world_master_key(endpoint.secret_key(), &world_slug);
        world.set_world_master_key(world_master_key).await;
        info!("World master key source: derived from iroh identity and world slug");
        let pointer_mode = ma_pointer_mode_enabled();

        let endpoint_id = endpoint.id().to_string();
        let world_did = ensure_world_did_document(
            &kubo_url,
            &world_slug,
            &endpoint_id,
            world_master_key,
            pointer_mode,
        )
        .await?;
        world.set_world_did_root(&world_did).await?;
        info!("Runtime world DID: {}", world_did);

        let restore_root = if pointer_mode {
            resolve_world_root_cid_from_did_pointer(&kubo_url, &world_did).await?
        } else {
            resolve_world_root_cid_from_did_inline(&kubo_url, &world_did).await?
        };
        if let Some(root_cid) = restore_root {
            match world.load_from_world_cid(&root_cid).await {
                Ok(rooms_loaded) => info!(
                    "Restored world from DID ma runtime fields: root_cid={} rooms={}",
                    root_cid, rooms_loaded
                ),
                Err(err) => warn!(
                    "Failed restoring world from DID ma runtime fields {}: {}",
                    root_cid, err
                ),
            }

            if let Some(state_cid) = world.state_cid().await {
                match world.load_encrypted_state(&state_cid).await {
                    Ok(new_root_cid) => info!(
                        "Restored encrypted runtime state: state_cid={} root_cid={}",
                        state_cid, new_root_cid
                    ),
                    Err(err) => warn!(
                        "Failed restoring encrypted runtime state {}: {}",
                        state_cid, err
                    ),
                }
            }
        }

        if world.world_cid().await.is_none() {
            let (state_cid, root_cid) = world.save_encrypted_state().await?;
            info!(
                "Bootstrapped world state with lobby snapshot: state_cid={} root_cid={}",
                state_cid,
                root_cid
            );
        }

        {
            let world_for_washer = world.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(OBJECT_WASHER_INTERVAL_SECS));
                loop {
                    ticker.tick().await;
                    match world_for_washer.flush_dirty_object_blobs().await {
                        Ok(count) if count > 0 => {
                            debug!("object washer flushed {} dirty blobs", count);
                        }
                        Ok(_) => {}
                        Err(err) => {
                            warn!("object washer flush failed: {}", err);
                        }
                    }
                }
            });
        }

        let did_cache = Arc::new(RwLock::new(HashMap::new()));
        let router = Router::builder(endpoint.clone())
            .accept(
                WORLD_ALPN,
                WorldProtocol {
                    world: world.clone(),
                    endpoint: endpoint.clone(),
                    endpoint_id: endpoint_id.clone(),
                    did_cache: did_cache.clone(),
                    lane: WorldLane::World,
                },
            )
            .accept(
                CMD_ALPN,
                WorldProtocol {
                    world: world.clone(),
                    endpoint: endpoint.clone(),
                    endpoint_id: endpoint_id.clone(),
                    did_cache,
                    lane: WorldLane::Cmd,
                },
            )
            .accept(
                CHAT_ALPN,
                WorldProtocol {
                    world: world.clone(),
                    endpoint: endpoint.clone(),
                    endpoint_id: endpoint_id.clone(),
                    did_cache: Arc::new(RwLock::new(HashMap::new())),
                    lane: WorldLane::Chat,
                },
            )
            .accept(
                CLOSET_ALPN,
                ClosetProtocol {
                    world: world.clone(),
                },
            )
            .spawn();
        let online_started = Instant::now();
        let online_status = match tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await {
            Ok(_) => format!("ok in {}ms", online_started.elapsed().as_millis()),
            Err(_) => format!("timeout after {}ms", online_started.elapsed().as_millis()),
        };
        let endpoint_addr = endpoint.addr();

        let direct_addresses = endpoint_addr
            .ip_addrs()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>();
        let multiaddrs = endpoint_addr
            .ip_addrs()
            .map(socket_addr_to_multiaddr)
            .collect::<Vec<_>>();
        let relay_urls = endpoint_addr
            .relay_urls()
            .map(|url| url.to_string())
            .collect::<Vec<_>>();

        let world_info = WorldInfo {
            name: world_slug.clone(),
            world_did: world_did.clone(),
            status_url: status_url.clone(),
            endpoint_id: endpoint_id.clone(),
            direct_addresses,
            multiaddrs,
            relay_urls,
            kubo_url: kubo_url.clone(),
            location_hint: format!("/iroh/{endpoint_id}"),
            entry_acl: world.entry_acl_source().await,
            started_at: Utc::now().to_rfc3339(),
            capabilities: vec![
                LaneCapability::for_lane(WorldLane::World),
                LaneCapability::for_lane(WorldLane::Cmd),
                LaneCapability::for_lane(WorldLane::Chat),
            ],
            actor_web: None,
        };

        let mut world_info = world_info;
        if let Some((source_dir, version, cid)) = actor_web_runtime.clone() {
            let actor_listener = bind_status_listener(&actor_web_listen).await?;
            let actor_addr = actor_listener.local_addr()?;
            let actor_status_url = format!("http://{}", actor_addr);
            let actor_info = ActorWebInfo {
                version,
                cid,
                status_url: actor_status_url.clone(),
                source_dir: source_dir.display().to_string(),
            };
            world_info.actor_web = Some(actor_info);

            tokio::spawn(async move {
                if let Err(err) = status::serve_actor_web(actor_listener, source_dir).await {
                    error!("actor web server failed: {}", err);
                }
            });
        }

        let status_world = world.clone();
        let status_info = world_info.clone();
        tokio::spawn(async move {
            if let Err(err) = status::serve(listener, status_world, status_info).await {
                error!("status server failed: {}", err);
            }
        });

        info!("Created default room: {}", DEFAULT_ROOM);
        info!("World endpoint id: {}", world_info.endpoint_id);
        info!("World status page: {}", world_info.status_url);
        info!("World protocol ALPN: {}", String::from_utf8_lossy(WORLD_ALPN));
        info!("Command protocol ALPN: {}", String::from_utf8_lossy(CMD_ALPN));
        info!("Chat protocol ALPN: {}", String::from_utf8_lossy(CHAT_ALPN));
        info!("Closet protocol ALPN: {}", String::from_utf8_lossy(CLOSET_ALPN));
        info!("Broadcast protocol ALPN (outbound push to agents): {}", String::from_utf8_lossy(BROADCAST_ALPN));
        info!("World entry ACL: {}", world_info.entry_acl);
        info!("Optional DID field ma:presenceHint = {}", world_info.location_hint);
        info!("Iroh online readiness: {}", online_status);
        if let Some(actor_web) = &world_info.actor_web {
            info!(
                "Actor web runtime: {} (dir={}, version={})",
                actor_web.status_url,
                actor_web.source_dir,
                actor_web.version.as_deref().unwrap_or("unknown")
            );
        } else {
            info!("Actor web runtime disabled (set actor_web_dir in runtime config to enable)");
        }

        for relay_url in &world_info.relay_urls {
            let probe = probe_relay(relay_url).await;
            info!("Relay probe {} -> {}", relay_url, probe);
        }

        println!("\n╔══════════════════════════════════════════════════════════╗");
        println!("║ ma-world Server                                         ║");
        println!("║ status page:   {:<41} ║", trim_console(&world_info.status_url, 41));
        println!("║ kubo API:      {:<41} ║", trim_console(&world_info.kubo_url, 41));
        println!("╚══════════════════════════════════════════════════════════╝");
        println!("world endpoint full: {}\n", world_info.endpoint_id);

        world
            .record_event(format!("world online at {}", world_info.status_url))
            .await;
        if !world.is_unlocked().await {
            world
                .record_event("world runtime locked; unlock required before handling commands".to_string())
                .await;
        }
        world
            .record_event(format!("iroh online readiness: {}", online_status))
            .await;
        world
            .record_event(format!("world direct addrs: {}", world_info.direct_addresses.join(", ")))
            .await;
        world
            .record_event(format!("world relays: {}", world_info.relay_urls.join(", ")))
            .await;
        for relay_url in &world_info.relay_urls {
            let probe = probe_relay(relay_url).await;
            world
                .record_event(format!("relay probe {} -> {}", relay_url, probe))
                .await;
        }
        world
            .record_event(format!("entry acl: {}", world_info.entry_acl))
            .await;
        world
            .record_event(format!("optional published location hint: {}", world_info.location_hint))
            .await;
        world
            .record_event(format!("world protocol ready on ALPN {}", String::from_utf8_lossy(WORLD_ALPN)))
            .await;
        world
            .record_event(format!("command protocol ready on ALPN {}", String::from_utf8_lossy(CMD_ALPN)))
            .await;
        world
            .record_event(format!("chat protocol ready on ALPN {}", String::from_utf8_lossy(CHAT_ALPN)))
            .await;

        info!("World initialized. Waiting for connections...");
        tokio::signal::ctrl_c().await?;
        info!("Shutting down ma-world — saving state...");

        match world.save_encrypted_state().await {
            Ok((state_cid, root_cid)) => {
                info!("State saved: state_cid={} root_cid={}", state_cid, root_cid);
            }
            Err(e) => {
                warn!("Failed to save state on shutdown: {}", e);
            }
        }

        router.shutdown().await?;
        Ok(())
    }
    .await;

    endpoint.close().await;
    run_result
}

async fn bind_status_listener(listen_addr: &str) -> Result<TcpListener> {
    let socket: SocketAddr = listen_addr
        .parse()
        .map_err(|e| anyhow!("invalid --listen '{}': {}", listen_addr, e))?;
    TcpListener::bind(socket)
        .await
        .map_err(|e| anyhow!("failed to bind status listener on {}: {}", socket, e))
}

fn format_system_time(time: SystemTime) -> String {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

fn socket_addr_to_multiaddr(addr: &SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) => format!("/ip4/{}/udp/{}/quic-v1", ip, addr.port()),
        IpAddr::V6(ip) => format!("/ip6/{}/udp/{}/quic-v1", ip, addr.port()),
    }
}

fn trim_console(input: &str, width: usize) -> String {
    let mut output = input.chars().take(width).collect::<String>();
    if output.len() < width {
        output.push_str(&" ".repeat(width - output.len()));
    }
    output
}

async fn probe_relay(relay_url: &str) -> String {
    let started = Instant::now();
    let client = match reqwest::Client::builder().timeout(RELAY_PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(err) => return format!("client-build-error: {}", err),
    };

    match client.get(relay_url).send().await {
        Ok(resp) => format!("http {} in {}ms", resp.status().as_u16(), started.elapsed().as_millis()),
        Err(err) => format!("error {} in {}ms", err, started.elapsed().as_millis()),
    }
}

fn resolve_avatar_inbox(did: &Did) -> Result<String> {
    did.fragment
        .clone()
        .ok_or_else(|| anyhow!("sender DID must include #fragment (local avatar inbox atom)"))
}

fn resolve_avatar_inbox_from_session(did: &Did, session_fragment: Option<&str>) -> Result<String> {
    if let Some(fragment) = did.fragment.clone() {
        return Ok(fragment);
    }

    let fallback = session_fragment
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    fallback.ok_or_else(|| {
        anyhow!(
            "sender DID must include #fragment (local avatar inbox atom); run apply in closet to assign alias/fragment first"
        )
    })
}

fn load_entry_acl() -> Result<EntryAcl> {
    let raw = std::env::var(WORLD_ENTRY_ACL_ENV).unwrap_or_else(|_| DEFAULT_ENTRY_ACL.to_string());
    parse_entry_acl(&raw)
}

fn parse_entry_acl(raw: &str) -> Result<EntryAcl> {
    let tokens = raw
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return Err(anyhow!(
            "{} is empty; use '*' or comma separated did:ma:... values",
            WORLD_ENTRY_ACL_ENV
        ));
    }

    let mut allow_all = false;
    let mut allowed_dids = HashSet::new();
    let mut owner_token_present = false;
    for token in tokens {
        if token == "*" {
            allow_all = true;
            continue;
        }
        if token.eq_ignore_ascii_case("owner") {
            owner_token_present = true;
            continue;
        }

        let did = Did::try_from(token)?;
        allowed_dids.insert(did.without_fragment().id());
    }

    if !allow_all && allowed_dids.is_empty() && !owner_token_present {
        return Err(anyhow!(
            "{} must contain '*', 'owner', or at least one valid DID",
            WORLD_ENTRY_ACL_ENV
        ));
    }

    Ok(EntryAcl {
        allow_all,
        allow_owner: owner_token_present,
        allowed_dids,
        source: raw.trim().to_string(),
    })
}

fn load_persisted_iroh_secret_key(path: &PathBuf) -> Result<Option<SecretKey>> {
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(path)?;
    let key_bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid iroh secret key file length in {}", path.display()))?;

    Ok(Some(SecretKey::from_bytes(&key_bytes)))
}

fn generate_iroh_secret_file(path: &PathBuf) -> Result<()> {
    if path.exists() {
        return Err(anyhow!("iroh secret already exists at {}", path.display()));
    }

    if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() {
            // Relative file in current directory, no directory to create.
        } else {
            fs::create_dir_all(parent)?;
        }
    }

    let mut key_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key_bytes);
    fs::write(path, key_bytes)?;

    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    }

    Ok(())
}

