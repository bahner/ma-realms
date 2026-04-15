#![forbid(unsafe_code)]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};


use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bootstrap::{
    load_runtime_file_config, print_cli_help, runtime_config_path,
    runtime_iroh_secret_default_path, xdg_data_home,
};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use chrono::Utc;
use did_ma::{DID_PREFIX, Did, Document, EncryptionKey, Message, SigningKey, VerificationMethod};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream},
    endpoint::presets,
    protocol::{AcceptError, ProtocolHandler, Router},
};
use ma_core::{
    ActorCommand, AVATAR_ALPN, PRESENCE_ALPN,
    CONTENT_TYPE_BROADCAST, CONTENT_TYPE_EVENT, CONTENT_TYPE_PRESENCE, CONTENT_TYPE_WORLD, CompiledCapabilityAcl,
    DEFAULT_WORLD_RELAY_URL,
    ExitData, LaneCapability, MessageEnvelope, ObjectDefinition, ObjectInboxMessage,
    MAILBOX_COMMANDS_INLINE,
    IpfsPublishDidResponse,
    ObjectMessageIntent, ObjectMessageKind, ObjectMessageRetention, ObjectMessageTarget,
    ObjectRuntimeState, IPFS_ALPN, PresenceAvatar, RoomActorAction, RoomActorContext,
    RoomEvent, TransportAck, TransportAckCode, INBOX_ALPN, WorldCommand, WorldLane, WorldRequest,
    WorldResponse, compile_acl, create_world_did, evaluate_compiled_acl_with_owner,
    execute_room_actor_command,
    normalize_spoken_text, parse_capability_acl_text, parse_object_local_capability_acl,
    parse_property_command, parse_property_command_for_keys,
    Reply, Scope,
    LegacyRequirement, RequirementChecker, RequirementSet, RequirementValue,
    pin_update_add_rm,
    TtlCache,
    expand_tilde_path, format_system_time, is_valid_nanoid_id, parse_rfc3339_unix,
    extract_did_description_from_json, normalize_language_for_did_document,
    sender_encryption_pubkey_multibase_from_document, sender_profile_from_document,
    sender_push_endpoint_from_document,
    generate_iroh_secret_file, load_persisted_iroh_secret_key, socket_addr_to_multiaddr,
    SecureFileKind, ensure_private_dir, write_secure_file,
};
use moka::sync::Cache;
use nanoid::nanoid;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{net::TcpListener, sync::{Mutex, RwLock}};
use tracing::{debug, error, info, warn};
use tracing_subscriber::prelude::*;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

mod actor;
mod avatar_commands;
mod bootstrap;
mod content_validation;
mod kubo;
mod lang;
mod room;
mod schema;
mod status;

use actor::{Avatar, AvatarRequest};
use lang::{
    collapse_world_language_order_strict,
    supported_world_languages_text,
    tr_world,
    world_lang_from_profile,
};
use kubo::{
    IpnsPublishOptions, dag_get_dag_cbor, dag_put_dag_cbor, generate_kubo_key, ipfs_add,
    ipns_publish_with_retry, list_kubo_key_names, list_kubo_keys, pin_add_named,
    pin_rm, wait_for_kubo_api,
};
use room::{Room, RoomAcl};
use schema::{
    ActorSecretBundle, AvatarRegistryEntry, AvatarStateDoc, ExitYamlDoc, IpldLink,
    LegacyRoomYaml, PersistedWorldEnvelope, RoomAclDoc, RoomStateDoc, RoomYamlDocV2,
    RuntimeStateDoc, WorldRootIndex, WorldRootIndexDag, WorldRootPrivateDag,
    WorldRootPublicDag, WorldRootRoomDagValue, WorldRootRoomEntry,
    default_world_dir, did_fragment, load_world_authoring,
    normalize_world_key_name, unlock_actor_secret_bundles, validate_world_authoring,
};
use status::{AvatarSnapshot, RoomSnapshot, WorldInfo, WorldSnapshot};

const DEFAULT_ROOM: &str = "lobby";
const DEFAULT_ENTRY_ACL: &str = "*";
const WORLD_ENTRY_ACL_ENV: &str = "MA_WORLD_ENTRY_ACL";
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:5002";
const MAX_EVENTS: usize = 200;
const MAX_KNOCK_INBOX: usize = 512;
const MAX_AVATAR_LOCATION_CACHE: usize = 8192;
const KNOCK_PENDING_TTL_SECS: i64 = 24 * 60 * 60;
const KNOCK_DECIDED_TTL_SECS: i64 = 60 * 60;
#[allow(dead_code)]
const MAX_OBJECT_INBOX: usize = 512;
const OBJECT_INBOX_INDEX_CAPACITY: u64 = 4096;
const MAILBOX_LOCK_SECS: u64 = 600;
const OBJECT_WASHER_INTERVAL_SECS: u64 = 20;
const IPNS_PUBLISH_INTERVAL_SECS_DEFAULT: u64 = 600;
const PRESENCE_PROBE_INTERVAL_SECS_DEFAULT: u64 = 5;
const PRESENCE_STALE_AFTER_SECS_DEFAULT: u64 = 30;
const WORLD_PING_INTERVAL_SECS: u32 = 5;
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

#[derive(Clone, Debug, Serialize)]
struct WorldBroadcastEnvelope {
    v: u8,
    kind: String,
    room: String,
    room_did: String,
    message: String,
    ts: String,
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
struct PresenceRefreshRequestEvent {
    v: u8,
    kind: String,
    room: String,
    room_did: String,
    ts: String,
}

#[derive(Clone, Debug, Serialize)]
struct PresenceRoomStateEvent {
    v: u8,
    kind: String,
    room: String,
    room_did: String,
    room_title: String,
    room_description: String,
    avatars: Vec<PresenceAvatar>,
    latest_event_sequence: u64,
    room_object_dids: HashMap<String, String>,
    ts: String,
}

#[derive(Clone, Debug, Serialize)]
struct RoomEventEnvelope {
    v: u8,
    kind: String,
    room: String,
    room_did: String,
    room_title: String,
    room_description: String,
    avatars: Vec<PresenceAvatar>,
    event: RoomEvent,
    latest_event_sequence: u64,
    ts: String,
}

#[derive(Clone, Debug)]
struct WorldProtocol {
    world: Arc<World>,
    endpoint: Endpoint,
    endpoint_id: String,
    did_cache: Arc<RwLock<HashMap<String, CachedDidDocument>>>,
    push_stream_cache: Arc<Mutex<HashMap<String, PushStreamCache>>>,
    push_timeout_cooldown: Arc<Mutex<HashMap<String, Instant>>>,
    lane: WorldLane,
}

#[derive(Debug)]
struct PushStreamCache {
    connection: Connection,
    send: SendStream,
    recv: RecvStream,
}

const PUSH_TIMEOUT_COOLDOWN_SECS: u64 = 8;

#[derive(Clone, Debug)]
struct IpfsProtocol {
    kubo_url: String,
    did_cache: Arc<RwLock<HashMap<String, CachedDidDocument>>>,
}

/// Placeholder protocol handler active while the world is locked.
///
/// Reads the first framed request and responds with a "world is locked" error
/// in the same length-prefixed JSON format the real protocols use, then closes.
#[derive(Clone, Debug)]
struct LockedGateProtocol;

impl ProtocolHandler for LockedGateProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;

        // Read first frame (we still need to consume it to follow the protocol).
        let frame_len = match recv.read_u32().await {
            Ok(n) => n as usize,
            Err(_) => {
                let _ = send.finish();
                return Ok(());
            }
        };
        if frame_len > 256 * 1024 {
            let _ = send.finish();
            return Ok(());
        }
        let mut buf = vec![0u8; frame_len];
        if recv.read_exact(&mut buf).await.is_err() {
            let _ = send.finish();
            return Ok(());
        }

        let response = serde_json::json!({
            "ok": false,
            "message": "world is locked; waiting for unlock"
        });
        let payload = serde_json::to_vec(&response).unwrap_or_default();
        let _ = send.write_u32(payload.len() as u32).await;
        let _ = send.write_all(&payload).await;
        let _ = send.flush().await;
        let _ = send.finish();
        Ok(())
    }
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

#[derive(Clone, Debug)]
struct InboxRoute {
    room_name: String,
    object_id: String,
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
            "inbox" => Some(RequirementValue::String(format!("room.{}.inbox", self.room_name))),
            _ => {
                if room::parse_room_inbox_symbol(symbol).is_some() {
                    return Some(RequirementValue::String(symbol.to_string()));
                }
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AvatarLocationEntry {
    did: String,
    room: String,
    endpoint: String,
    seen_at: String,
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
    /// Runtime-only index: root DID -> current room name.
    avatar_room_index: Arc<RwLock<HashMap<String, String>>>,
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
    /// IPNS key for this world (the identifier portion of `did:ma:<ipns>`).
    world_ipns: Arc<RwLock<Option<String>>>,
    /// Full world DID (with fragment) from authored world config.
    world_did: Arc<RwLock<Option<String>>>,
    /// Runtime state lock; when false, inbox ingress rejects world interactions.
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
    /// IPNS publish tracking.
    last_publish_ok: Arc<RwLock<Option<bool>>>,
    last_publish_root_cid: Arc<RwLock<Option<String>>>,
    last_publish_error: Arc<RwLock<Option<String>>>,
    /// True when IPFS root has changed since last IPNS publish.
    ipns_dirty: Arc<RwLock<bool>>,
    /// Room-local interactable objects keyed by room then object id.
    room_objects: Arc<RwLock<HashMap<String, HashMap<String, ObjectRuntimeState>>>>,
    /// Inbox of async knock requests for private worlds.
    knock_inbox: Arc<RwLock<TtlCache<u64, KnockMessage>>>,
    /// Monotonic knock id sequence.
    next_knock_id: Arc<RwLock<u64>>,
    /// TTL-backed avatar location table keyed by root DID.
    avatar_locations: Arc<RwLock<TtlCache<String, AvatarLocationEntry>>>,
    /// Global avatar DID-doc registry keyed by avatar fragment.
    avatar_registry: Arc<RwLock<HashMap<String, AvatarRegistryEntry>>>,
    /// Fast lookup from object DID to room/object inbox route.
    object_inbox_index: Cache<String, InboxRoute>,
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

fn normalize_local_object_id(input: &str) -> String {
    input
        .trim()
        .trim_start_matches('#')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
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

fn format_public_inspect_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "(null)".to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
    }
}

fn resolve_public_inspect_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let trimmed = path.trim().trim_matches('.');
    if trimmed.is_empty() || trimmed == "_list" {
        return Some(value);
    }

    let mut current = value;
    for segment in trimmed.split('.') {
        let key = segment.trim();
        if key.is_empty() {
            return None;
        }
        match current {
            serde_json::Value::Object(map) => current = map.get(key)?,
            serde_json::Value::Array(items) => {
                let index: usize = key.parse().ok()?;
                current = items.get(index)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

impl World {
    pub(crate) fn new(entry_acl: EntryAcl, kubo_url: String, world_root_pin_name: String) -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_EVENTS))),
            room_events: Arc::new(RwLock::new(HashMap::new())),
            next_room_event_sequence: Arc::new(RwLock::new(0)),
            entry_acl: Arc::new(RwLock::new(entry_acl)),
            handle_to_did: Arc::new(RwLock::new(HashMap::new())),
            did_to_handle: Arc::new(RwLock::new(HashMap::new())),
            avatar_room_index: Arc::new(RwLock::new(HashMap::new())),
            actor_secrets: Arc::new(RwLock::new(HashMap::new())),
            owner_did: Arc::new(RwLock::new(None)),
            kubo_url: Arc::new(RwLock::new(kubo_url)),
            room_cids: Arc::new(RwLock::new(HashMap::new())),
            world_cid: Arc::new(RwLock::new(None)),
            world_ipns: Arc::new(RwLock::new(None)),
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
            last_publish_ok: Arc::new(RwLock::new(None)),
            last_publish_root_cid: Arc::new(RwLock::new(None)),
            last_publish_error: Arc::new(RwLock::new(None)),
            ipns_dirty: Arc::new(RwLock::new(false)),
            room_objects: Arc::new(RwLock::new(HashMap::new())),
            knock_inbox: Arc::new(RwLock::new(TtlCache::with_capacity(
                Duration::from_secs(KNOCK_PENDING_TTL_SECS as u64),
                MAX_KNOCK_INBOX,
            ))),
            next_knock_id: Arc::new(RwLock::new(0)),
            avatar_locations: Arc::new(RwLock::new(TtlCache::with_capacity(
                Duration::from_secs(PRESENCE_STALE_AFTER_SECS_DEFAULT),
                MAX_AVATAR_LOCATION_CACHE,
            ))),
            avatar_registry: Arc::new(RwLock::new(HashMap::new())),
            object_inbox_index: Cache::new(OBJECT_INBOX_INDEX_CAPACITY),
        }
    }

    pub async fn world_root_pin_name(&self) -> String {
        self.world_root_pin_name.read().await.clone()
    }

    async fn set_avatar_description_for_did(
        &self,
        room_name: &str,
        did_id: &str,
        description: &str,
    ) -> bool {
        let mut rooms = self.rooms.write().await;
        let Some(room) = rooms.get_mut(room_name) else {
            return false;
        };

        let mut updated = false;
        for avatar in room.avatars.values_mut() {
            if avatar.agent_did.id() == did_id {
                avatar.set_description(description.to_string());
                updated = true;
            }
        }
        updated
    }

    async fn touch_avatar_presence_for_did(&self, room_name: &str, did_id: &str) -> bool {
        let mut rooms = self.rooms.write().await;
        let Some(room) = rooms.get_mut(room_name) else {
            return false;
        };

        let mut updated = false;
        for avatar in room.avatars.values_mut() {
            if avatar.agent_did.id() == did_id {
                avatar.touch_presence();
                updated = true;
            }
        }
        updated
    }

    async fn avatar_language_order_for_did(&self, room_name: &str, did_id: &str) -> Option<String> {
        let rooms = self.rooms.read().await;
        let room = rooms.get(room_name)?;
        room.avatars
            .values()
            .find(|avatar| avatar.agent_did.id() == did_id)
            .map(|avatar| avatar.language_order.clone())
            .filter(|value| !value.trim().is_empty())
    }

    async fn avatar_handle_for_did(&self, room_name: &str, did_id: &str) -> Option<String> {
        let rooms = self.rooms.read().await;
        let room = rooms.get(room_name)?;
        room
            .avatars
            .iter()
            .find(|(_, avatar)| avatar.agent_did.id() == did_id)
            .map(|(handle, _)| handle.clone())
    }

    async fn public_inspect_tree(&self) -> serde_json::Value {
        let world_did = self
            .world_did
            .read()
            .await
            .clone()
            .unwrap_or_else(|| format!("{DID_PREFIX}unconfigured"));
        let owner = self
            .owner_did
            .read()
            .await
            .clone()
            .unwrap_or_else(|| "(none)".to_string());
        let lang_cid = self
            .lang_cid
            .read()
            .await
            .clone()
            .unwrap_or_else(|| "(none)".to_string());
        let avatar_registry = self.avatar_registry.read().await.clone();
        let rooms = self.rooms.read().await;

        let mut rooms_json = serde_json::Map::new();
        for (room_name, room) in rooms.iter() {
            let mut avatars_json = serde_json::Map::new();
            for (handle, avatar) in room.avatars.iter() {
                avatars_json.insert(
                    handle.clone(),
                    serde_json::json!({
                        "did": avatar.agent_did.id(),
                        "owner": avatar.owner,
                        "description": avatar.description_or_default(),
                        "fragment": avatar.agent_did.fragment.clone().unwrap_or_default(),
                        "lang": avatar.language_order,
                        "endpoint": avatar.agent_endpoint,
                        "acl": avatar.acl.summary(),
                        "shortcuts": avatar.object_shortcuts,
                    }),
                );
            }

            rooms_json.insert(
                room_name.clone(),
                serde_json::json!({
                    "did": room.did,
                    "title": room.title_or_default(),
                    "description": room.description_or_default(),
                    "avatars": avatars_json,
                    "avatar_count": room.avatars.len(),
                }),
            );
        }
        drop(rooms);

        serde_json::json!({
            "did": world_did,
            "owner": owner,
            "rooms": rooms_json,
            "avatars": avatar_registry,
            "lang_cid": lang_cid,
        })
    }

    async fn refresh_avatar_registry_entry_for_did(&self, did_id: &str) -> Result<()> {
        let (room_name, avatar) = {
            let rooms = self.rooms.read().await;
            let mut found: Option<(String, Avatar)> = None;
            for (room_name, room) in rooms.iter() {
                if let Some(entry) = room
                    .avatars
                    .values()
                    .find(|avatar| avatar.agent_did.id() == did_id)
                {
                    found = Some((room_name.clone(), entry.clone()));
                    break;
                }
            }
            found.ok_or_else(|| anyhow!("avatar '{}' not found in world rooms", did_id))?
        };

        let avatar_did = avatar.agent_did.clone();
        let fragment = avatar_did
            .fragment
            .clone()
            .ok_or_else(|| anyhow!("avatar DID '{}' missing fragment", avatar_did.id()))?;
        let encryption_pubkey = avatar
            .encryption_pubkey_multibase
            .clone()
            .ok_or_else(|| anyhow!("avatar '{}' missing keyAgreement public key", avatar_did.id()))?;
        let description = avatar.description_or_default();
        let avatar_name = avatar.inbox.clone();
        let owner = avatar.owner.clone();
        let language_order = avatar.language_order.clone();
        let endpoint = avatar.agent_endpoint.clone();
        let acl_summary = avatar.acl.summary();
        let shortcuts = avatar.object_shortcuts.clone();

        let world_did_raw = self
            .world_did
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("world DID is not configured"))?;
        let world_did = Did::try_from(world_did_raw.as_str())
            .map_err(|e| anyhow!("invalid configured world DID '{}': {}", world_did_raw, e))?;

        let world_signing_key_bytes = self
            .unlocked_world_signing_key
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("world signing key is not unlocked"))?;
        let signer_did = Did::new_root(&world_did.ipns)
            .map_err(|e| anyhow!("failed building world signer DID: {}", e))?;
        let signing_key = SigningKey::from_private_key_bytes(signer_did, world_signing_key_bytes)
            .map_err(|e| anyhow!("failed restoring world signing key: {}", e))?;

        let mut document = Document::new(&avatar_did, &avatar_did);

        let assertion_vm = VerificationMethod::new(
            avatar_did.base_id(),
            avatar_did.base_id(),
            signing_key.key_type.clone(),
            "assertion",
            signing_key.public_key_multibase.clone(),
        )
        .map_err(|e| anyhow!("failed building avatar assertion method: {}", e))?;
        let key_agreement_vm = VerificationMethod::new(
            avatar_did.base_id(),
            avatar_did.base_id(),
            "Multikey",
            "key-agreement",
            encryption_pubkey.clone(),
        )
        .map_err(|e| anyhow!("failed building avatar keyAgreement method: {}", e))?;

        let assertion_vm_id = assertion_vm.id.clone();
        let key_agreement_vm_id = key_agreement_vm.id.clone();
        document
            .add_verification_method(assertion_vm.clone())
            .map_err(|e| anyhow!("failed adding avatar assertion method: {}", e))?;
        document
            .add_verification_method(key_agreement_vm)
            .map_err(|e| anyhow!("failed adding avatar keyAgreement method: {}", e))?;
        document.assertion_method = vec![assertion_vm_id];
        document.key_agreement = vec![key_agreement_vm_id];
        document.set_ma_type("avatar")?;
        if let Some(did_language_order) = normalize_language_for_did_document(&language_order) {
            if let Err(err) = document.set_language(did_language_order.clone()) {
                warn!(
                    "ignoring invalid avatar language '{}' for {}: {}",
                    did_language_order,
                    avatar_did.id(),
                    err
                );
            }
        }
        document.set_ma_transports(serde_json::Value::Array(
            vec![
                format!(
                    "/ma-iroh/{}/{}",
                    avatar.agent_endpoint,
                    String::from_utf8_lossy(PRESENCE_ALPN)
                ),
                format!(
                    "/ma-iroh/{}/{}",
                    avatar.agent_endpoint,
                    String::from_utf8_lossy(INBOX_ALPN)
                ),
            ]
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
        ));
        document.set_ma_ping_interval_secs(WORLD_PING_INTERVAL_SECS);
        document
            .sign(&signing_key, &assertion_vm)
            .map_err(|e| anyhow!("failed signing avatar DID document: {}", e))?;

        let kubo_url = self.kubo_url().await;
        let document_json = document
            .marshal()
            .map_err(|e| anyhow!("failed marshaling avatar DID document: {}", e))?;
        let document_value: serde_json::Value = serde_json::from_str(&document_json)
            .map_err(|e| anyhow!("failed converting avatar DID document to JSON value: {}", e))?;
        dag_put_dag_cbor(&kubo_url, &document_value).await?;

        let next_entry = AvatarRegistryEntry {
            did: avatar_did.id(),
            name: avatar_name,
            description,
            owner,
            fragment: fragment.clone(),
            lang: language_order,
            endpoint,
            room: room_name,
            key_agreement: encryption_pubkey,
            acl: acl_summary,
            shortcuts,
            identity: IpldLink {
                cid: format!("/ipns/{}", avatar_did.ipns),
            },
        };

        let changed = {
            let mut registry = self.avatar_registry.write().await;
            let changed = registry.get(&fragment).map(|entry| {
                entry.did != next_entry.did
                    || entry.name != next_entry.name
                    || entry.description != next_entry.description
                    || entry.owner != next_entry.owner
                    || entry.fragment != next_entry.fragment
                    || entry.lang != next_entry.lang
                    || entry.endpoint != next_entry.endpoint
                    || entry.room != next_entry.room
                    || entry.key_agreement != next_entry.key_agreement
                    || entry.acl != next_entry.acl
                    || entry.shortcuts != next_entry.shortcuts
                    || entry.identity.cid != next_entry.identity.cid
            }).unwrap_or(true);
            registry.insert(fragment, next_entry);
            changed
        };

        if changed {
            let _ = self.save_world_index().await;
        }

        Ok(())
    }

    /// Find the avatar DID owned by `owner_did` anywhere in this world.
    /// Returns `None` if the owner has no avatar (i.e. has not entered).
    async fn resolve_avatar_did_for_owner(&self, owner_did: &str) -> Option<Did> {
        let rooms = self.rooms.read().await;
        for (_room_name, room) in rooms.iter() {
            for (_handle, avatar) in room.avatars.iter() {
                if avatar.owner == owner_did {
                    return Some(avatar.agent_did.clone());
                }
            }
        }
        None
    }

    /// Ensure sender has an avatar in the given room.
    /// Creates the avatar on first contact; refreshes endpoint/presence for existing ones.
    /// Returns (avatar_did, handle, newly_created).
    async fn ensure_avatar(
        &self,
        sender_did: &Did,
        sender_profile: &str,
        agent_endpoint: &str,
        sender_encryption_pubkey_multibase: &str,
        room: &str,
    ) -> Result<(Did, String, bool)> {
        let is_new = self.resolve_avatar_did_for_owner(&sender_did.base_id()).await.is_none();

        if is_new && !self.can_enter(sender_did).await {
            return Err(anyhow!("entry denied by ACL for {}", sender_did.id()));
        }

        let language_order = collapse_world_language_order_strict(sender_profile)
            .ok_or_else(|| anyhow!(
                "no supported language found in ma.language='{}'. supported={}",
                sender_profile,
                supported_world_languages_text()
            ))?;

        let world_ipns = self
            .local_world_ipns()
            .await
            .unwrap_or_else(|| "unconfigured".to_string());
        let avatar_fragment = sender_did
            .fragment
            .clone()
            .unwrap_or_else(|| "avatar".to_string())
            .trim()
            .trim_start_matches('@')
            .to_string();
        let avatar_did =
            Did::try_from(create_world_did(&world_ipns, &avatar_fragment).as_str())
                .map_err(|err| anyhow!("invalid derived avatar DID: {}", err))?;

        let signing_key = SigningKey::generate(avatar_did.clone())
            .map_err(|e| anyhow!("failed to generate avatar signing key: {}", e))?;

        let avatar_req = AvatarRequest {
            did: avatar_did.clone(),
            owner: sender_did.base_id(),
            agent_endpoint: agent_endpoint.to_string(),
            language_order,
            signing_secret: signing_key.private_key_bytes(),
            encryption_pubkey_multibase: Some(sender_encryption_pubkey_multibase.to_string()),
        };

        let handle = self.join_room(room, avatar_req, None).await?;

        if is_new {
            let _ = self
                .set_avatar_description_for_did(room, &avatar_did.id(), "skeleton avatar")
                .await;
        }

        Ok((avatar_did, handle, is_new))
    }

    async fn upsert_avatar_location(&self, room_name: &str, did: &str, endpoint: &str) {
        let entry = AvatarLocationEntry {
            did: did.to_string(),
            room: room_name.to_string(),
            endpoint: endpoint.to_string(),
            seen_at: Utc::now().to_rfc3339(),
        };
        self.avatar_locations
            .write()
            .await
            .insert(did.to_string(), entry);
    }

    async fn remove_avatar_location(&self, did: &str) {
        self.avatar_locations
            .write()
            .await
            .remove(&did.to_string());
    }

    async fn rebuild_avatar_room_index(&self) {
        let rooms = self.rooms.read().await;
        let mut next = HashMap::new();
        for (room_name, room) in rooms.iter() {
            for avatar in room.avatars.values() {
                next.insert(avatar.agent_did.id(), room_name.clone());
            }
        }
        drop(rooms);
        *self.avatar_room_index.write().await = next;
    }

    async fn find_avatar_presence_by_did(
        &self,
        did_query: &Did,
    ) -> Option<(String, String, String, String, String)> {
        let query_id = did_query.id();

        let rooms = self.rooms.read().await;
        for (room_name, room) in rooms.iter() {
            for (handle, avatar) in room.avatars.iter() {
                if avatar.agent_did.id() != query_id {
                    continue;
                }
                return Some((
                    room_name.clone(),
                    handle.clone(),
                    avatar.agent_did.id(),
                    avatar.agent_endpoint.clone(),
                    avatar.description_or_default(),
                ));
            }
        }
        None
    }

    async fn did_description_fallback(&self, did_query: &Did) -> Option<String> {
        let kubo_url = self.kubo_url().await;
        let document = kubo::fetch_did_document(&kubo_url, did_query).await.ok()?;
        let raw = document.marshal().ok()?;
        extract_did_description_from_json(&raw)
    }

    async fn avatar_room_for_did(&self, did_id: &str) -> Option<String> {
        let indexed_room = self.avatar_room_index.read().await.get(did_id).cloned();
        if let Some(room_name) = indexed_room {
            let rooms = self.rooms.read().await;
            let valid = rooms
                .get(room_name.as_str())
                .map(|room| {
                    room
                        .avatars
                        .values()
                        .any(|avatar| avatar.agent_did.id() == did_id)
                })
                .unwrap_or(false);
            drop(rooms);
            if valid {
                return Some(room_name);
            }
        }

        let discovered = {
            let rooms = self.rooms.read().await;
            rooms
                .iter()
                .find(|(_, room)| {
                    room
                        .avatars
                        .values()
                        .any(|avatar| avatar.agent_did.id() == did_id)
                })
                .map(|(room_name, _)| room_name.clone())
        };

        let mut index = self.avatar_room_index.write().await;
        if let Some(room_name) = discovered.clone() {
            index.insert(did_id.to_string(), room_name);
        } else {
            index.remove(did_id);
        }
        discovered
    }

    async fn prune_stale_avatars(&self, stale_after: Duration) -> Vec<String> {
        let now = SystemTime::now();
        let mut changed_rooms = Vec::new();
        let mut removed_dids: Vec<String> = Vec::new();

        let mut rooms = self.rooms.write().await;
        for (room_name, room) in rooms.iter_mut() {
            let stale_handles = room
                .avatars
                .iter()
                .filter_map(|(handle, avatar)| {
                    let age = now
                        .duration_since(avatar.last_seen_at)
                        .unwrap_or_else(|_| Duration::from_secs(0));
                    if age > stale_after {
                        Some(handle.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            if stale_handles.is_empty() {
                continue;
            }

            for handle in stale_handles {
                if let Some(avatar) = room.avatars.remove(&handle) {
                    removed_dids.push(avatar.agent_did.id());
                    info!(
                        "[{}] removed stale avatar {} (endpoint={}, stale_after={}s)",
                        room_name,
                        handle,
                        avatar.agent_endpoint,
                        stale_after.as_secs()
                    );
                }
            }

            changed_rooms.push(room_name.clone());
        }

        drop(rooms);

        if !changed_rooms.is_empty() {
            self.rebuild_avatar_room_index().await;
            for removed_did in removed_dids {
                self.remove_avatar_location(&removed_did).await;
            }
        }

        changed_rooms
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
        let ipns = self
            .local_world_ipns()
            .await
            .unwrap_or_else(|| "unconfigured".to_string());
        let objects = self.room_objects.read().await;
        let Some(room_map) = objects.get(room_name) else {
            return HashMap::new();
        };

        let mut out = HashMap::new();
        for object in room_map.values() {
            let object_did = create_world_did(&ipns, &object.id);
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
            if !self.is_local_world_ipns(&did.ipns).await {
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

    async fn resolve_inbox_target_object_id(&self, room_name: &str, target: &str) -> Option<String> {
        let normalized = target.trim();
        if normalized.eq_ignore_ascii_case(":inbox") || normalized.eq_ignore_ascii_case("inbox") {
            let objects = self.room_objects.read().await;
            let room_map = objects.get(room_name)?;
            if room_map.contains_key("mailbox") {
                return Some("mailbox".to_string());
            }
            return room_map
                .values()
                .find(|object| {
                    object.has_receiver_role("world-inbox") || object.has_receiver_protocol("ma/inbox/1")
                })
                .map(|object| object.id.clone());
        }

        if let Some(token) = room::parse_room_inbox_symbol(normalized) {
            return self.resolve_room_object_id(room_name, token).await;
        }

        None
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
        caller_did: &str,
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
            (object.owner.clone(), object.state.clone())
        };

        let world_owner = self.owner_did.read().await.clone();

        let global_match = {
            let global_acl = self.global_capability_acl.read().await;
            match global_acl.as_ref() {
                None => true,
                Some(acl) => evaluate_compiled_acl_with_owner(
                    acl,
                    caller_did,
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
                caller_did,
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
                        caller_did,
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

    pub async fn set_world_did(&self, world_did: &str) -> Result<()> {
        let parsed = Did::try_from(world_did)
            .map_err(|e| anyhow!("invalid world DID '{}': {}", world_did, e))?;
        let ipns = parsed.ipns.clone();
        let bare = parsed.base_id();

        *self.world_ipns.write().await = Some(ipns.clone());
        *self.world_did.write().await = Some(bare.clone());

        // Keep runtime rooms aligned with the configured world IPNS.
        // This fixes stale values like did:ma:unconfigured#lobby created before DID bootstrap.
        {
            let mut rooms = self.rooms.write().await;
            for (room_name, room) in rooms.iter_mut() {
                room.did = create_world_did(&ipns, room_name);
            }
        }

        // Bootstrap owner identity from the world DID when owner has not
        // been explicitly restored yet (e.g. first boot or missing runtime state).
        // This keeps entry ACL policy-driven while avoiding owner lockout.
        let owner_missing = self.owner_did.read().await.is_none();
        if owner_missing {
            *self.owner_did.write().await = Some(bare.clone());
            self.allow_entry_did(&bare).await;
        }

        Ok(())
    }

    async fn local_world_ipns(&self) -> Option<String> {
        self.world_ipns.read().await.clone()
    }

    async fn build_room_did(&self, room_id: &str) -> String {
        let ipns = self
            .local_world_ipns()
            .await
            .unwrap_or_else(|| "unconfigured".to_string());
        create_world_did(&ipns, room_id)
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

    async fn is_local_world_ipns(&self, ipns: &str) -> bool {
        self.world_ipns
            .read()
            .await
            .as_ref()
            .map(|local| local == ipns)
            .unwrap_or(false)
    }

    async fn is_world_target_did(&self, target: &str) -> bool {
        let target = target.trim();
        if target.is_empty() {
            return false;
        }

        let _configured_ipns = self.world_ipns.read().await.clone();
        let configured_full = self.world_did.read().await.clone();

        // Strict match against configured full DID.
        if configured_full
            .as_deref()
            .map(|full| full == target)
            .unwrap_or(false)
        {
            return true;
        }

        // Postel-tolerant: accept configured DID root as @world alias.
        if configured_full
            .as_deref()
            .and_then(|full| full.split('#').next())
            .map(|root| root == target)
            .unwrap_or(false)
        {
            return true;
        }

        false
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
        let did_id = did.id();
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
                .is_some_and(|owner| owner == &did_id)
        {
            return true;
        }
        acl.allowed_dids.contains(&did_id)
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

    pub async fn allow_entry_did(&self, did: &str) {
        let mut acl = self.entry_acl.write().await;
        acl.allowed_dids.insert(did.to_string());
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

    fn parse_invite_did_arg(target_did_raw: &str) -> Result<String, String> {
        Did::try_from(target_did_raw)
            .map(|did| did.id())
            .map_err(|err| format!("invalid DID '{}': {}", target_did_raw, err))
    }

    fn lookup_object_print_method(
        object: &ObjectRuntimeState,
        method: &str,
        _sender_profile: &str,
    ) -> Option<String> {
        let verbs = object.definition.as_ref().map(|def| &def.verbs)?;

        let needle = method.trim().to_ascii_lowercase();
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

    fn lookup_object_method_definition(
        object: &ObjectRuntimeState,
        method: &str,
    ) -> Option<ma_core::ObjectVerbDefinition> {
        let verbs = object.definition.as_ref().map(|def| &def.verbs)?;
        let needle = method.trim().to_ascii_lowercase();
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
        self.prune_knock_inbox().await;
        let mut inbox = self.knock_inbox.write().await;
        if let Some(existing) = inbox
            .items_any()
            .into_iter()
            .map(|(_, item)| item)
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

        while inbox.len_any() >= MAX_KNOCK_INBOX {
            let _ = inbox.pop_first_any();
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

        inbox.insert(id, knock.clone());
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

    async fn prune_knock_inbox(&self) -> usize {
        let now = Utc::now().timestamp();
        let mut inbox = self.knock_inbox.write().await;
        let stale_ids = inbox
            .items_any()
            .into_iter()
            .filter_map(|(id, item)| {
                let requested_ts = parse_rfc3339_unix(&item.requested_at).unwrap_or(now);
                let keep = if item.status == KnockStatus::Pending {
                    now.saturating_sub(requested_ts) <= KNOCK_PENDING_TTL_SECS
                } else {
                    let decided_ts = item
                        .decided_at
                        .as_deref()
                        .and_then(parse_rfc3339_unix)
                        .unwrap_or(requested_ts);
                    now.saturating_sub(decided_ts) <= KNOCK_DECIDED_TTL_SECS
                };
                if keep {
                    None
                } else {
                    Some(*id)
                }
            })
            .collect::<Vec<_>>();

        let removed = stale_ids.len();
        for id in stale_ids {
            let _ = inbox.remove(&id);
        }
        removed
    }

    async fn flush_knock_inbox(&self) -> usize {
        let mut inbox = self.knock_inbox.write().await;
        let removed = inbox.len_any();
        inbox.clear();
        removed
    }

    async fn list_knocks(&self, pending_only: bool) -> Vec<KnockMessage> {
        self.prune_knock_inbox().await;
        let inbox = self.knock_inbox.read().await;
        inbox
            .items_any()
            .into_iter()
            .map(|(_, item)| item)
            .filter(|item| !pending_only || item.status == KnockStatus::Pending)
            .cloned()
            .collect()
    }

    async fn accept_knock(&self, id: u64) -> Result<KnockMessage> {
        self.prune_knock_inbox().await;
        let (accepted, requester_did) = {
            let mut inbox = self.knock_inbox.write().await;
            let Some(item) = inbox.get_mut_any(&id) else {
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
        self.prune_knock_inbox().await;
        let mut inbox = self.knock_inbox.write().await;
        let Some(item) = inbox.get_mut_any(&id) else {
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
        self.prune_knock_inbox().await;
        let mut inbox = self.knock_inbox.write().await;
        if inbox.remove(&id).is_none() {
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
                    let avatars = if !dag.public.avatars.is_empty() {
                        dag.public.avatars.clone()
                    } else {
                        dag.avatars.clone()
                    };
                    let state_cid = dag
                        .private
                        .as_ref()
                        .and_then(|private| private.state_cid.clone())
                        .or(dag.state_cid.clone());
                    let lang_cid = dag.public.lang_cid.clone().or(dag.lang_cid.clone());

                    *self.avatar_registry.write().await = avatars;
                    *self.state_cid.write().await = state_cid;
                    *self.lang_cid.write().await = lang_cid;
                    let mut had_embedded = false;
                    let room_entries = if !dag.public.rooms.is_empty() {
                        dag.public.rooms
                    } else {
                        dag.rooms
                    };
                    let rooms = room_entries
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
                    self.avatar_registry.write().await.clear();
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
                        if room_entry.acl_cid.trim().is_empty() {
                            return Err(anyhow!(
                                "missing acl_cid metadata for room '{}' in world index {}",
                                room_name,
                                root_cid
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
                        loaded_acl.owner = room_entry.owner.clone();
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
        // Backfill static snapshots for runtime rooms that don't yet have a room CID,
        // so the world root DAG remains browseable via ipfs dag get.
        let runtime_room_names = {
            let rooms = self.rooms.read().await;
            let mut names = rooms.keys().cloned().collect::<Vec<_>>();
            names.sort();
            names
        };
        let missing_room_cids = {
            let room_cids = self.room_cids.read().await;
            runtime_room_names
                .into_iter()
                .filter(|name| !room_cids.contains_key(name))
                .collect::<Vec<_>>()
        };
        for room_name in missing_room_cids {
            let cid = self.save_room_static(&room_name).await?;
            info!(
                "Room '{}' static snapshot backfilled as {} before world index save",
                room_name,
                cid
            );
        }

        let room_cids = self.room_cids.read().await.clone();
        let room_meta: HashMap<String, (String, String, String, Option<String>, RoomAcl)> = self
            .rooms
            .read()
            .await
            .iter()
            .map(|(name, room)| {
                (
                    name.clone(),
                    (
                        room.did.clone(),
                        room.title_or_default(),
                        room.description_or_default(),
                        room.acl.owner.clone(),
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

            let (did, title, description, owner_did, mut acl) = room_meta
                .get(&name)
                .cloned()
                .unwrap_or_else(|| (String::new(), String::new(), String::new(), None, RoomAcl::open()));

            // Owner is persisted inline in room entry, not in ACL doc.
            acl.owner = None;

            let acl_doc = RoomAclDoc {
                kind: "ma_room_acl".to_string(),
                version: 1,
                acl,
            };
            let acl_yaml = serde_yaml::to_string(&acl_doc)
                .map_err(|e| anyhow!("failed to serialize ACL for room '{}': {}", name, e))?;
            let acl_cid = kubo::ipfs_add(&kubo_url, acl_yaml.into_bytes()).await?;

            rooms_index.insert(
                name.clone(),
                WorldRootRoomDagValue::Entry(WorldRootRoomEntry {
                    cid,
                    name: Some(name.clone()),
                    title: if title.trim().is_empty() { None } else { Some(title) },
                    description: if description.trim().is_empty() { None } else { Some(description) },
                    did: if did.trim().is_empty() { None } else { Some(did) },
                    owner: owner_did,
                    acl_cid,
                    owner_cid: None,
                }),
            );
        }

        let index = WorldRootIndexDag {
            config: None,
            public: WorldRootPublicDag {
                rooms: rooms_index,
                avatars: self.avatar_registry.read().await.clone(),
                lang_cid: self.lang_cid.read().await.clone(),
            },
            private: Some(WorldRootPrivateDag {
                state_cid: self.state_cid.read().await.clone(),
            }),
            rooms: HashMap::new(),
            avatars: HashMap::new(),
            state_cid: None,
            lang_cid: None,
        };
        let new_cid = kubo::dag_put_dag_cbor(&kubo_url, &index).await?;

        // Keep exactly one named recursive pin for the world root index.
        let kubo_url_for_pin = kubo_url.clone();
        let kubo_url_for_unpin = kubo_url.clone();
        let pin_outcome = pin_update_add_rm(
            previous_world_cid.as_deref(),
            &new_cid,
            &pin_name,
            |cid, name| {
                let kubo_url = kubo_url_for_pin.clone();
                async move { pin_add_named(&kubo_url, &cid, &name).await }
            },
            |cid| {
                let kubo_url = kubo_url_for_unpin.clone();
                async move { pin_rm(&kubo_url, &cid).await }
            },
        )
        .await?;
        if let Some(rm_err) = pin_outcome.previous_unpin_error {
            if let Some(old_cid) = previous_world_cid.as_deref() {
                warn!("pin/rm failed for previous world root {}: {}", old_cid, rm_err);
            }
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
        let did_id = req.did.id();
        let previous_room = self.avatar_room_for_did(&did_id).await;

        let mut rooms = self.rooms.write().await;
        let room_acl_allows = rooms
            .get(room_name)
            .ok_or_else(|| anyhow!("Room {} not found", room_name))?
            .acl
            .can_enter(&req.did.id());

        // Check room-level ACL.
        if !room_acl_allows {
            return Err(anyhow!("room ACL denied entry for {}", req.did.id()));
        }

        // Enforce unique room membership per DID by moving from previous room when needed.
        if let Some(prev_room_name) = previous_room
            .as_ref()
            .filter(|value| value.as_str() != room_name)
        {
            let moved = if let Some(prev_room) = rooms.get_mut(prev_room_name.as_str()) {
                let previous_handle = prev_room
                    .avatars
                    .iter()
                    .find(|(_, avatar)| avatar.agent_did.id() == did_id)
                    .map(|(handle, _)| handle.clone());
                previous_handle.and_then(|handle| prev_room.avatars.remove(handle.as_str()))
            } else {
                None
            };

            if let Some(mut avatar) = moved {
                let endpoint_changed = avatar.agent_endpoint != req.agent_endpoint;
                let language_changed = avatar.language_order != req.language_order;
                let owner_changed = avatar.owner != req.owner;
                let encryption_changed = avatar.encryption_pubkey_multibase != req.encryption_pubkey_multibase;
                avatar.agent_endpoint = req.agent_endpoint.clone();
                avatar.language_order = req.language_order.clone();
                avatar.owner = req.owner.clone();
                avatar.encryption_pubkey_multibase = req.encryption_pubkey_multibase.clone();
                avatar.touch_presence();
                let moved_handle = avatar.inbox.clone();
                let refresh_registry = endpoint_changed || language_changed || owner_changed || encryption_changed;

                if let Some(room) = rooms.get_mut(room_name) {
                    room.add_avatar(avatar);
                }
                drop(rooms);
                self.rebuild_avatar_room_index().await;
                self
                    .upsert_avatar_location(room_name, &did_id, &req.agent_endpoint)
                    .await;
                if refresh_registry {
                    if let Err(err) = self.refresh_avatar_registry_entry_for_did(&did_id).await {
                        warn!("failed refreshing avatar registry for {}: {}", did_id, err);
                    }
                }

                info!(
                    "[{}] {} moved from {} ({:?})",
                    room_name,
                    moved_handle,
                    prev_room_name,
                    req.did
                );
                self.record_event(format!(
                    "[{room_name}] {} moved from {} with {}",
                    moved_handle,
                    prev_room_name,
                    req.did.id(),
                ))
                .await;
                self.record_room_event(
                    room_name,
                    "system",
                    Some(moved_handle.clone()),
                    Some(did_id.clone()),
                    Some(req.agent_endpoint.clone()),
                    format!("{} entered {}", moved_handle, room_name),
                )
                .await;
                return Ok(moved_handle);
            }
        }

        let room = rooms
            .get_mut(room_name)
            .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

        // Same DID already present? Refresh endpoint/presence and return existing handle.
        if let Some((existing_handle, existing)) = room
            .avatars
            .iter_mut()
            .find(|(_, avatar)| avatar.agent_did.id() == did_id)
        {
            let endpoint_changed = existing.agent_endpoint != req.agent_endpoint;
            let language_changed = existing.language_order != req.language_order;
            let owner_changed = existing.owner != req.owner;
            let encryption_changed = existing.encryption_pubkey_multibase != req.encryption_pubkey_multibase;
            existing.agent_endpoint = req.agent_endpoint.clone();
            existing.language_order = req.language_order.clone();
            existing.owner = req.owner.clone();
            existing.encryption_pubkey_multibase = req.encryption_pubkey_multibase.clone();
            existing.touch_presence();
            info!("[{}] {} already present ({:?})", room_name, existing_handle, req.did);
            let existing_handle_value = existing_handle.clone();
            let refresh_registry = endpoint_changed || language_changed || owner_changed || encryption_changed;
            drop(rooms);
            self.rebuild_avatar_room_index().await;
            self
                .upsert_avatar_location(room_name, &did_id, &req.agent_endpoint)
                .await;
            if refresh_registry {
                if let Err(err) = self.refresh_avatar_registry_entry_for_did(&did_id).await {
                    warn!("failed refreshing avatar registry for {}: {}", did_id, err);
                }
            }
            return Ok(existing_handle_value);
        }

        drop(rooms);

        let did_fragment = req.did.fragment.clone().unwrap_or_default();
        let handle = self
            .register_handle(&did_id, preferred_handle, &did_fragment)
            .await;

        let avatar = Avatar::new(
            handle.clone(),
            req.did.clone(),
            req.agent_endpoint.clone(),
            req.language_order.clone(),
            req.owner.clone(),
            req.signing_secret,
            req.encryption_pubkey_multibase.clone(),
        );

        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_name)
            .ok_or_else(|| anyhow!("Room {} not found", room_name))?;
        room.add_avatar(avatar);
        drop(rooms);
        self.rebuild_avatar_room_index().await;
        self
            .upsert_avatar_location(room_name, &did_id, &req.agent_endpoint)
            .await;
        if let Err(err) = self.refresh_avatar_registry_entry_for_did(&did_id).await {
            warn!("failed refreshing avatar registry for {}: {}", did_id, err);
        }

        info!("[{}] {} joined ({:?}) from {}", room_name, handle, req.did, req.agent_endpoint);
        self.record_event(format!(
            "[{room_name}] {} joined with {} from {}",
            handle,
            req.did.id(),
            req.agent_endpoint
        ))
        .await;
        self.record_room_event(
            room_name,
            "system",
            Some(handle.clone()),
            Some(did_id.clone()),
            Some(req.agent_endpoint.clone()),
            format!("{} entered {}", handle, room_name),
        )
        .await;
        Ok(handle)
    }

    /// Assign or recover a world-unique handle for `did`.
    /// The preferred_handle (from the client) or inbox fragment is the starting candidate.
    /// On collision with a different DID, appends the last 4 characters of the DID.
    async fn register_handle(
        &self,
        did: &str,
        preferred: Option<String>,
        fragment: &str,
    ) -> String {
        let mut h2d = self.handle_to_did.write().await;
        let mut d2h = self.did_to_handle.write().await;

        // Same DID already has a handle? Return it, normalizing legacy '@' prefixes.
        if let Some(existing) = d2h.get(did).cloned() {
            let normalized = existing.trim().trim_start_matches('@').to_string();
            if !normalized.is_empty() && normalized != existing {
                h2d.remove(existing.as_str());
                h2d.insert(normalized.clone(), did.to_string());
                d2h.insert(did.to_string(), normalized.clone());
                return normalized;
            }
            return existing;
        }

        let preferred_norm = preferred
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_start_matches('@').to_string())
            .filter(|value| is_valid_nanoid_id(value.as_str()));
        let fragment_norm = fragment
            .trim()
            .trim_start_matches('@')
            .to_string();

        let mut candidates: Vec<String> = Vec::new();
        if let Some(name) = preferred_norm {
            candidates.push(name);
        }
        if is_valid_nanoid_id(fragment_norm.as_str())
            && !candidates.iter().any(|entry| entry == &fragment_norm)
        {
            candidates.push(fragment_norm);
        }
        candidates.push(did.to_string());

        for candidate in candidates {
            match h2d.get(candidate.as_str()) {
                Some(owner) if owner != did => continue,
                _ => {
                    h2d.insert(candidate.clone(), did.to_string());
                    d2h.insert(did.to_string(), candidate.clone());
                    return candidate;
                }
            }
        }

        let fallback = did.to_string();
        h2d.insert(fallback.clone(), did.to_string());
        d2h.insert(did.to_string(), fallback.clone());
        fallback
    }

    /// Broadcast a signed chat message to room event log.
    pub async fn send_chat(
        &self,
        room_name: &str,
        sender_handle: &str,
        sender_did: &str,
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
            sender_did: Some(sender_did.to_string()),
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

        let removed_did_root = room
            .avatars
            .get(actor_name)
            .map(|avatar| avatar.agent_did.id());
        room.remove_avatar(actor_name);
        drop(rooms);
        self.rebuild_avatar_room_index().await;
        if let Some(removed_did) = removed_did_root {
            self.remove_avatar_location(&removed_did).await;
        }

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
        let sender_did_id = from_did.id();
        let from_norm = from.trim().trim_start_matches('@').to_string();
        let sender_presence_required = match &envelope {
            MessageEnvelope::ActorCommand { target, .. } => {
                let normalized = target.trim().to_ascii_lowercase();
                matches!(
                    normalized.as_str(),
                    "avatar" | "me" | "self" | "here" | "room" | "world"
                )
            }
            _ => true,
        };
        let sender_key = {
            let rooms = self.rooms.read().await;
            let room = rooms
                .get(room_name)
                .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

            if !sender_presence_required {
                if let Some((handle, avatar)) = room
                    .avatars
                    .iter()
                    .find(|(_, avatar)| avatar.agent_did.id() == sender_did_id)
                {
                    if avatar.agent_did.id() != sender_did_id {
                        return Err(anyhow!(
                            "sender DID mismatch for @{} in room {}",
                            from_norm,
                            room_name
                        ));
                    }
                    handle.clone()
                } else if !from_norm.is_empty() {
                    from_norm.clone()
                } else {
                    sender_did_id.clone()
                }
            } else {

                if let Some(avatar) = room.avatars.get(from_norm.as_str()) {
                    if avatar.agent_did.id() == sender_did_id {
                        from_norm.clone()
                    } else {
                        return Err(anyhow!(
                            "sender DID mismatch for @{} in room {}",
                            from_norm,
                            room_name
                        ));
                    }
                } else {
                    if let Some((handle, avatar)) = room
                        .avatars
                        .iter()
                        .find(|(_, avatar)| avatar.agent_did.id() == sender_did_id)
                    {
                        if avatar.agent_did.id() != sender_did_id {
                            return Err(anyhow!(
                                "sender DID mismatch for @{} in room {}",
                                from_norm,
                                room_name
                            ));
                        }
                        handle.clone()
                    } else {
                        return Err(anyhow!("unknown avatar @{} in room {}", from_norm, room_name));
                    }
                }
            }
        };

        if sender_presence_required {
            let rooms = self.rooms.read().await;
            let room = rooms
                .get(room_name)
                .ok_or_else(|| anyhow!("Room {} not found", room_name))?;

            let Some(avatar) = room.avatars.get(&sender_key) else {
                return Err(anyhow!("unknown avatar @{} in room {}", from_norm, room_name));
            };
            if avatar.agent_did.id() != sender_did_id {
                return Err(anyhow!(
                    "sender DID mismatch for @{} in room {}",
                    from_norm,
                    room_name
                ));
            }
        }

        let (response, broadcasted, effective_room) = match envelope {
            MessageEnvelope::Chatter { text } => {
                let speech = normalize_spoken_text(&text);
                debug!("[{}] {}: {}", room_name, sender_key, speech);
                self.record_event(format!("[{room_name}] {sender_key}: {speech}")).await;
                let rendered = format!("{}: {}", sender_key, speech);
                self.record_room_event(room_name, "speech", Some(sender_key.clone()), Some(from_did.id()), None, speech.clone())
                    .await;
                (rendered, true, room_name.to_string())
            }
            MessageEnvelope::RoomCommand { command } => {
                let caller_did = from_did.id();
                let broadcasted = command.starts_with("say ") || command.starts_with("emote ");
                let response = self
                    .room_command(room_name, &command, &sender_key, sender_profile, Some(caller_did.as_str()))
                    .await;
                debug!("[{}] {} -> @here: {} -> {}", room_name, sender_key, command, response);
                self.record_event(format!("[{room_name}] {sender_key} -> @here: {command} => {}", response))
                    .await;
                (response, broadcasted, room_name.to_string())
            }
            MessageEnvelope::ActorCommand { target, command } => {
                let broadcasted = matches!(command, ActorCommand::Say { .. } | ActorCommand::Emote { .. });
                let (response, effective_room) = self
                    .handle_actor_command(room_name, &sender_key, from_did, sender_profile, &target, command)
                    .await;
                self.rebuild_avatar_room_index().await;
                debug!("[{}] {} -> @{} -> {}", room_name, sender_key, target, response);
                self.record_event(format!(
                    "[{room_name}] {sender_key} -> @{target} => {}",
                    response.replace('\n', " ")
                ))
                .await;
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
        let active_dids: HashSet<String> = {
            let locations = self.avatar_locations.read().await;
            locations
                .items()
                .into_iter()
                .filter_map(|(did_root, entry)| {
                    if entry.room == room_name {
                        Some(did_root.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };
        let rooms = self.rooms.read().await;
        let Some(room) = rooms.get(room_name) else { return Vec::new() };
        let mut avatars: Vec<PresenceAvatar> = room.avatars.iter()
            .filter(|(_, avatar)| {
                let did_id = avatar.agent_did.id();
                active_dids.contains(&did_id)
            })
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
        let bare = parsed.base_id();
        *self.owner_did.write().await = Some(bare.clone());
        self.allow_entry_did(&bare).await;
        info!("World owner set to {}", bare);
        Ok(bare)
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

    pub async fn last_publish_status(&self) -> (Option<bool>, Option<String>, Option<String>) {
        (
            self.last_publish_ok.read().await.clone(),
            self.last_publish_root_cid.read().await.clone(),
            self.last_publish_error.read().await.clone(),
        )
    }

    pub async fn save_encrypted_state(&self) -> Result<(String, String)> {
        let flushed = self.flush_dirty_object_blobs().await?;
        if flushed > 0 {
            info!("flushed {} dirty object blobs before save", flushed);
        }

        let kubo_url = self.kubo_url().await;
        let secrets = self.read_world_runtime_secrets().await?;
        let world_did_str = self
            .world_did.read().await.clone()
            .ok_or_else(|| anyhow!("world DID is not configured"))?;
        let world_did = Did::try_from(world_did_str.as_str())
            .map_err(|e| anyhow!("invalid configured world DID '{}': {}", world_did_str, e))?;
        let signer_did = Did::new_root(&world_did.ipns)
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
                        encryption_pubkey_multibase: avatar.encryption_pubkey_multibase.clone(),
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
        let avatar_registry = self.avatar_registry.read().await.clone();

        let state = RuntimeStateDoc {
            kind: "ma_world_runtime_state".to_string(),
            version: 1,
            rooms: rooms_snapshot,
            events,
            room_events,
            next_room_event_sequence,
            handle_to_did,
            did_to_handle,
            owner: owner_did,
            room_cids,
            room_objects,
            avatar_registry,
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

        *self.ipns_dirty.write().await = true;

        Ok((state_cid, root_cid))
    }

    /// Publish the current world state to IPNS. Updates the DID document with the
    /// latest world root CID as an IPLD link and publishes to the world IPNS key.
    pub async fn publish_to_ipns(&self) -> Result<()> {
        let kubo_url = self.kubo_url().await;
        let secrets = self.read_world_runtime_secrets().await?;
        let state_cid = self.state_cid.read().await.clone()
            .ok_or_else(|| anyhow!("no state CID available for IPNS publish"))?;
        let root_cid = self.world_cid.read().await.clone()
            .ok_or_else(|| anyhow!("no world root CID available for IPNS publish"))?;

        if let Err(err) = publish_world_did_runtime_ma(
            &kubo_url,
            &self.world_root_pin_name().await,
            secrets.world_master_key,
            &state_cid,
            &root_cid,
        )
        .await
        {
            let message = err.to_string();
            *self.last_publish_ok.write().await = Some(false);
            *self.last_publish_root_cid.write().await = Some(root_cid.clone());
            *self.last_publish_error.write().await = Some(message.clone());
            return Err(anyhow!("IPNS publish failed: {}", message));
        }

        *self.last_publish_ok.write().await = Some(true);
        *self.last_publish_root_cid.write().await = Some(root_cid);
        *self.last_publish_error.write().await = None;
        *self.ipns_dirty.write().await = false;

        Ok(())
    }

    /// Save state to IPFS and immediately publish to IPNS.
    pub async fn save_and_publish(&self) -> Result<(String, String)> {
        let result = self.save_encrypted_state().await?;
        if let Err(err) = self.publish_to_ipns().await {
            warn!("IPNS publish after save failed: {}", err);
        }
        Ok(result)
    }

    /// Returns true if IPFS state has changed since last IPNS publish.
    pub async fn is_ipns_dirty(&self) -> bool {
        *self.ipns_dirty.read().await
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
        let world_did_str = self
            .world_did.read().await.clone()
            .ok_or_else(|| anyhow!("world DID is not configured"))?;
        let world_did = Did::try_from(world_did_str.as_str())
            .map_err(|e| anyhow!("invalid configured world DID '{}': {}", world_did_str, e))?;
        let signer_did = Did::new_root(&world_did.ipns)
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
                    avatar_doc.owner.clone(),
                    [0u8; 32], // Restored avatars have no signing key — agent must re-Enter.
                    avatar_doc.encryption_pubkey_multibase,
                );
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
        *self.avatar_registry.write().await = state.avatar_registry;
        let loaded_owner_did = state.owner;
        *self.owner_did.write().await = loaded_owner_did.clone();
        if let Some(owner) = loaded_owner_did {
            self.allow_entry_did(&owner).await;
        }
        *self.room_cids.write().await = state.room_cids;
        *self.lang_cid.write().await = state.lang_cid;
        *self.state_cid.write().await = Some(state_cid.to_string());
        self.rebuild_avatar_room_index().await;

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
        let target = target.trim();
        let target_lower = target.to_ascii_lowercase();
        if matches!(target_lower.as_str(), "@here" | "here" | "room" | "@world" | "world" | "@me" | "me" | "self" | "@avatar" | "avatar") {
            return (
                "actor-local aliases (@here/@world/@me/@avatar) must be resolved to DID before send".to_string(),
                room_name.to_string(),
            );
        }

        let caller_did = from_did.id();
        if let Ok(target_did) = Did::try_from(target) {

            if self.is_local_world_ipns(&target_did.ipns).await {
                if target_did.fragment.is_none() || self.is_world_target_did(target).await {
                    let cmd = match &command {
                        ActorCommand::Say { payload } => format!("say {}", payload.trim()),
                        ActorCommand::Emote { payload } => format!("emote {}", payload.trim()),
                        ActorCommand::Raw { command } => command.trim().to_string(),
                    };
                    let effective_cmd = if cmd.is_empty() { "_list".to_string() } else { cmd };
                    return (
                        self
                            .handle_world_command(room_name, from, from_did, sender_profile, &effective_cmd)
                            .await,
                        room_name.to_string(),
                    );
                }

                if let Some(fragment) = target_did.fragment.clone() {
                    let room_exists = {
                        let rooms = self.rooms.read().await;
                        rooms.contains_key(fragment.as_str())
                    };
                    if room_exists {
                        let room_cmd = match &command {
                            ActorCommand::Say { payload } => format!("say {}", payload.trim()),
                            ActorCommand::Emote { payload } => format!("emote {}", payload.trim()),
                            ActorCommand::Raw { command } => command.trim().to_string(),
                        };
                        let effective_cmd = if room_cmd.is_empty() {
                            "show".to_string()
                        } else {
                            room_cmd
                        };
                        return (
                            self
                                .room_command(
                                    fragment.as_str(),
                                    effective_cmd.as_str(),
                                    from,
                                    sender_profile,
                                    Some(caller_did.as_str()),
                                )
                                .await,
                            fragment,
                        );
                    }
                }
            }

            if target_did.id() == caller_did {
                return self
                    .handle_avatar_command(room_name, from, from_did, sender_profile, command)
                    .await;
            }

            // Postel's law: actor sent its own DID (not the world-derived avatar DID).
            // Check if any avatar in the room is owned by target_did → treat as self-targeting.
            {
                let rooms = self.rooms.read().await;
                if let Some(room) = rooms.get(room_name) {
                    let is_owner_match = room.avatars.get(from)
                        .map(|a| a.owner == target_did.base_id())
                        .unwrap_or(false);
                    if is_owner_match {
                        drop(rooms);
                        return self
                            .handle_avatar_command(room_name, from, from_did, sender_profile, command)
                            .await;
                    }
                }
            }
        }

                if let Ok(target_did) = Did::try_from(target.trim()) {
                    if let ActorCommand::Raw { command: raw } = &command {
                        if raw.trim().eq_ignore_ascii_case("debug") {
                            if let Some((room, handle, did, endpoint, description)) =
                                self.find_avatar_presence_by_did(&target_did).await
                            {
                                return (
                                    format!(
                                        "@debug kind=avatar\n@debug did={}\n@debug room={}\n@debug handle={}\n@debug endpoint={}\n@debug description={}",
                                        did,
                                        room,
                                        handle,
                                        endpoint,
                                        description
                                    ),
                                    room_name.to_string(),
                                );
                            }

                            if self.is_local_world_ipns(&target_did.ipns).await {
                                if let Some(fragment) = target_did.fragment.clone() {
                                    let world_owner = self.owner_did.read().await.clone();
                                    let objects = self.room_objects.read().await;
                                    for (candidate_room, room_map) in objects.iter() {
                                        if let Some(object) = room_map.get(fragment.as_str()) {
                                            let owner = object
                                                .owner
                                                .clone()
                                                .or(world_owner.clone())
                                                .unwrap_or_else(|| "(none)".to_string());
                                            return (
                                                format!(
                                                    "@debug kind=object\n@debug did={}\n@debug room={}\n@debug object_id={}\n@debug name={}\n@debug type={}\n@debug owner={}\n@debug cid={}\n@debug holder={}\n@debug opened_by={}\n@debug durable={}\n@debug persistence={}",
                                                    target_did.id(),
                                                    candidate_room,
                                                    object.id,
                                                    object.name,
                                                    object.kind,
                                                    owner,
                                                    object.cid.clone().unwrap_or_else(|| "(builtin)".to_string()),
                                                    object.holder.clone().unwrap_or_else(|| "(none)".to_string()),
                                                    object.opened_by.clone().unwrap_or_else(|| "(closed)".to_string()),
                                                    object.durable,
                                                    format!("{:?}", object.persistence).to_ascii_lowercase(),
                                                ),
                                                room_name.to_string(),
                                            );
                                        }
                                    }
                                    drop(objects);

                                    let rooms = self.rooms.read().await;
                                    for (candidate_room, room) in rooms.iter() {
                                        if room.did == target_did.id() || room.name == fragment {
                                            return (
                                                format!(
                                                    "@debug kind=room\n@debug did={}\n@debug room={}\n@debug title={}\n@debug description={}\n@debug avatars={}\n@debug exits={}",
                                                    room.did,
                                                    candidate_room,
                                                    room.title_or_default(),
                                                    room.description_or_default(),
                                                    room.avatars.len(),
                                                    room.exits.len(),
                                                ),
                                                room_name.to_string(),
                                            );
                                        }
                                    }
                                }
                            }

                            return (
                                format!("@debug target not found for {}", target_did.id()),
                                room_name.to_string(),
                            );
                        }
                    }
                }

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
                    let target_id = did.id();
                    if let Some((handle, _)) = room
                        .avatars
                        .iter()
                        .find(|(_, avatar)| avatar.agent_did.id() == target_id)
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
                    ActorCommand::Emote { payload } => {
                        let action = normalize_spoken_text(&payload);
                        (format!("* @{} {} at @{}", from, action, actor_target), room_name.to_string())
                    }
                    ActorCommand::Raw { command } => {
                        (format!("@{} is here. Try '@{} say \"...\"'. (got: {})", actor_target, actor_target, command), room_name.to_string())
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
        let caller_did = from_did.id();
        let now_secs = Utc::now().timestamp().max(0) as u64;
        let mut active_room = room_name.to_string();

        let resolved_target = if let Some(inbox_object_id) = self
            .resolve_inbox_target_object_id(room_name, target)
            .await
        {
            inbox_object_id
        } else if let Ok(did) = Did::try_from(target.trim()) {
            if !self.is_local_world_ipns(&did.ipns).await {
                return None;
            }
            let did_key = did.id().to_ascii_lowercase();
            if let Some(route) = self.object_inbox_index.get(&did_key) {
                let objects = self.room_objects.read().await;
                let valid = objects
                    .get(route.room_name.as_str())
                    .map(|room_map| {
                        room_map.contains_key(route.object_id.as_str())
                    })
                    .unwrap_or(false);
                if valid {
                    active_room = route.room_name.clone();
                    route.object_id
                } else {
                    self.object_inbox_index.invalidate(&did_key);
                    let fragment = did.fragment.clone()?;
                    let discovered_room = objects
                        .iter()
                        .find_map(|(candidate_room, room_map)| {
                            if room_map.contains_key(fragment.as_str()) {
                                Some(candidate_room.clone())
                            } else {
                                None
                            }
                        });
                    drop(objects);

                    if let Some(found_room) = discovered_room {
                        active_room = found_room.clone();
                        self.object_inbox_index.insert(
                            did_key,
                            InboxRoute {
                                room_name: found_room,
                                object_id: fragment.clone(),
                            },
                        );
                    }
                    fragment
                }
            } else {
                let fragment = did.fragment.clone()?;
                let objects = self.room_objects.read().await;
                let discovered_room = objects
                    .iter()
                    .find_map(|(candidate_room, room_map)| {
                        if room_map.contains_key(fragment.as_str()) {
                            Some(candidate_room.clone())
                        } else {
                            None
                        }
                    });
                drop(objects);

                if let Some(found_room) = discovered_room {
                    active_room = found_room.clone();
                    self.object_inbox_index.insert(
                        did_key,
                        InboxRoute {
                            room_name: found_room,
                            object_id: fragment.clone(),
                        },
                    );
                }
                fragment
            }
        } else {
            let token = target.trim().trim_start_matches('@').to_ascii_lowercase();
            let objects = self.room_objects.read().await;
            let room_map = objects.get(active_room.as_str())?;
            room_map
                .values()
                .find(|obj| obj.matches_target(token.as_str()))
                .map(|obj| obj.id.clone())?
        };

        let room_name = active_room.as_str();

        let object_id = {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            if let Some(device) = room_map.get_mut(&resolved_target) {
                device.clear_expired_lock(now_secs);
            }
            resolved_target
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
            ActorCommand::Emote { payload } => format!("emote {}", payload),
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
        let method = parts.next().unwrap_or("help").to_ascii_lowercase();

        let verb_requirements = {
            let objects = self.room_objects.read().await;
            let room_map = objects.get(room_name)?;
            let object = room_map.get(&object_id)?;
            Self::lookup_object_method_definition(object, &method)
                .map(|entry| entry.requirements)
                .unwrap_or_default()
        };

        let cap_verb = match method.as_str() {
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
            .object_capability_allowed(room_name, &object_id, &caller_did, &required_capability)
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
                    user: caller_did.clone(),
                    owner: object.owner.clone().or_else(|| world_owner.clone()),
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
            Self::lookup_object_print_method(object, &method, sender_profile)
        };
        if let Some(output) = declarative {
            return Some((output, room_name.to_string()));
        }

        if method == "help" {
            return Some((
                format!("{} commands: {}", object_label, MAILBOX_COMMANDS_INLINE),
                room_name.to_string(),
            ));
        }

        if let Some(property) = parse_property_command_for_keys(
            trimmed,
            &[
                "_list",
                "did",
                "kind",
                "name",
                "owner",
                "cid",
                "content-b64",
                "holder",
                "opened_by",
                "durable",
                "persistence",
                "durable_inbox_messages",
                "ephemeral_inbox_messages",
                "outbound_messages",
                "pending_messages",
            ],
        ) {
            let key = property.key;
            let value = property.value.unwrap_or_default();

            let (
                device_name,
                device_kind,
                object_did,
                cid,
                holder,
                opened_by,
                durable,
                persistence,
                durable_inbox_messages,
                ephemeral_inbox_messages,
                outbound_messages,
                owner,
            ) = {
                let world_owner = self.owner_did.read().await.clone();
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                let device = room_map.get(&object_id)?;
                let world_ipns = self
                    .local_world_ipns()
                    .await
                    .unwrap_or_else(|| "unconfigured".to_string());
                (
                    device.name.clone(),
                    device.kind.clone(),
                    create_world_did(&world_ipns, &device.id),
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
                    device
                        .owner
                        .clone()
                        .or(world_owner)
                        .unwrap_or_else(|| "(none)".to_string()),
                )
            };
            let pending_messages = self.list_knocks(true).await.len();

            if key == "_list" {
                return Some((
                    format!(
                        "@ .object.did {}\n@ .object.kind {}\n@ .object.name {}\n@ .object.owner {}\n@ .object.cid {}\n@ .object.holder {}\n@ .object.opened_by {}\n@ .object.durable {}\n@ .object.persistence {}\n@ .object.durable_inbox_messages {}\n@ .object.ephemeral_inbox_messages {}\n@ .object.outbound_messages {}\n@ .object.pending_messages {}",
                        object_did,
                        device_kind,
                        device_name,
                        owner,
                        cid,
                        holder,
                        opened_by,
                        durable,
                        persistence,
                        durable_inbox_messages,
                        ephemeral_inbox_messages,
                        outbound_messages,
                        pending_messages,
                    ),
                    room_name.to_string(),
                ));
            }

            if value.is_empty() {
                let response = match key.as_str() {
                    "did" => object_did,
                    "kind" => device_kind,
                    "name" => device_name,
                    "owner" => owner,
                    "cid" => cid,
                    "holder" => holder,
                    "opened_by" => opened_by,
                    "durable" => durable.to_string(),
                    "persistence" => persistence,
                    "durable_inbox_messages" => durable_inbox_messages.to_string(),
                    "ephemeral_inbox_messages" => ephemeral_inbox_messages.to_string(),
                    "outbound_messages" => outbound_messages.to_string(),
                    "pending_messages" => pending_messages.to_string(),
                    "content-b64" => "(write-only)".to_string(),
                    _ => format!("unknown object attribute '{}'", key),
                };
                return Some((response, room_name.to_string()));
            }

            let (object_name, object_owner, is_world_owner) = {
                let owner = self.owner_did.read().await.clone();
                let objects = self.room_objects.read().await;
                let room_map = objects.get(room_name)?;
                let object = room_map.get(&object_id)?;
                (
                    object.name.clone(),
                    object.owner.clone(),
                    owner
                        .as_deref()
                        .map(|did| did == from_did.base_id().as_str())
                        .unwrap_or(false),
                )
            };

            let is_object_owner = object_owner
                .as_deref()
                .map(|did| did == from_did.base_id().as_str())
                .unwrap_or(false);
            if !is_object_owner && !is_world_owner {
                return Some((
                    format!("only @{} owner or world owner can change object definition", object_name),
                    room_name.to_string(),
                ));
            }

            if key == "cid" {
                let (cid, definition, published_from_yaml) = match self.resolve_object_cid_or_yaml_input(value.as_str()).await {
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

            if key == "content-b64" {
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
                format!("@{} attribute '{}' is read-only", device_name, key),
                room_name.to_string(),
            ));
        }

        if method == "set" {
            return Some((
                format!("{} 'set ...' is deprecated. Use dot notation: @target.cid <cid|base64-yaml> or @target.content-b64 <base64-yaml>", object_label),
                room_name.to_string(),
            ));
        }

        if matches!(method.as_str(), "show" | "status" | "look" | "describe" | "apply") {
            return Some((
                format!("{} '{}' is deprecated. Use dot notation: @target._list or @target.<did|kind|name|owner|cid|holder|opened_by|durable|persistence|durable_inbox_messages|ephemeral_inbox_messages|outbound_messages|pending_messages>", object_label, method),
                room_name.to_string(),
            ));
        }

        if method == "take" || method == "pickup" || method == "hold" {
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

        if method == "drop" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if device.holder.as_deref() != Some(from) {
                return Some((format!("You are not holding @{}.", device.name), room_name.to_string()));
            }
            device.holder = None;
            if device.opened_by.as_deref() == Some(caller_did.as_str()) {
                device.opened_by = None;
                device.locked_by = None;
                device.lock_expires_at = None;
            }
            device.state_dirty = true;
            return Some((format!("You drop @{}.", device.name), room_name.to_string()));
        }

        if method == "open" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if device.holder.as_deref() != Some(from) {
                return Some((format!("You must hold @{} before opening it.", device.name), room_name.to_string()));
            }
            if let Some(locked_by) = device.locked_by.as_deref() {
                if locked_by != caller_did {
                    return Some((format!("@{} is locked by {}.", device.name, locked_by), room_name.to_string()));
                }
            }
            device.opened_by = Some(caller_did.clone());
            device.locked_by = Some(caller_did.clone());
            device.lock_expires_at = Some(now_secs + MAILBOX_LOCK_SECS);
            device.state_dirty = true;
            return Some((format!("@{} opened for {}", device.name, caller_did), room_name.to_string()));
        }

        if method == "close" {
            let mut objects = self.room_objects.write().await;
            let room_map = objects.get_mut(room_name)?;
            let device = room_map.get_mut(&object_id)?;
            if device.opened_by.as_deref() != Some(caller_did.as_str()) {
                return Some((format!("@{} is not open for your DID.", device.name), room_name.to_string()));
            }
            device.opened_by = None;
            device.locked_by = None;
            device.lock_expires_at = None;
            device.state_dirty = true;
            return Some((format!("@{} closed.", device.name), room_name.to_string()));
        }

        if method == "flush" {
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

        if method == "list" {
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

        if method == "pop" {
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

        if method == "ask" {
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
                        session_id: Some(caller_did.clone()),
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

        if method == "retry" {
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

        if method == "reply" {
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
                    from_did: Some(caller_did.clone()),
                    from_object: None,
                    kind: ObjectMessageKind::Whisper,
                    body: text.to_string(),
                    sent_at: Utc::now().to_rfc3339(),
                    content_type: Some("application/x-ma-object-ephemeral-reply".to_string()),
                    session_id: Some(caller_did.clone()),
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

        if method == "pending" {
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

        if method == "accept" {
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

        if method == "reject" {
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

        if method == "invite" {
            let Some(target_did_raw) = parts.next() else {
                return Some((format!("usage: {} invite <did> [note]", object_label), room_name.to_string()));
            };
            let target_did = match Self::parse_invite_did_arg(target_did_raw) {
                Ok(root) => root,
                Err(err) => return Some((err, room_name.to_string())),
            };
            self.allow_entry_did(&target_did).await;
            return Some((
                format!("invited {} (allowlisted)", target_did),
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
        fn split_selector_key(path: &str) -> (Option<String>, String) {
            let raw = path.trim();
            if let Some(rest) = raw.strip_prefix('#') {
                if let Some((selector, key)) = rest.split_once('.') {
                    return (
                        Some(selector.trim().to_ascii_lowercase()),
                        key.trim().to_ascii_lowercase(),
                    );
                }
                return (Some(rest.trim().to_ascii_lowercase()), String::new());
            }
            (None, raw.to_ascii_lowercase())
        }

        match command {
            ActorCommand::Say { payload } => {
                // say/emote are room methods — redirect to room_command.
                let speech = normalize_spoken_text(&payload);
                let room_cmd = format!("say {}", speech);
                let response = self.room_command(room_name, &room_cmd, from, sender_profile, Some(&from_did.id())).await;
                (response, room_name.to_string())
            }
            ActorCommand::Emote { payload } => {
                let text = normalize_spoken_text(&payload);
                let room_cmd = format!("emote {}", text);
                let response = self.room_command(room_name, &room_cmd, from, sender_profile, Some(&from_did.id())).await;
                (response, room_name.to_string())
            }
            ActorCommand::Raw { command } => {
                let trimmed = command.trim();

                if trimmed.eq_ignore_ascii_case("ping") || trimmed.eq_ignore_ascii_case("ping?") {
                    return (
                        format!("pong room={} handle={}", room_name, from),
                        room_name.to_string(),
                    );
                }

                if let Some(rest) = trimmed.strip_prefix("prop ") {
                    let Some(property) = parse_property_command(rest) else {
                        return (
                            "@avatar usage: @avatar.<name|description|owner|fragment|lang> [value] | @avatar.#<selector>.<name|description|owner|fragment|lang>".to_string(),
                            room_name.to_string(),
                        );
                    };

                    let path = property.key;
                    let value = property.value.unwrap_or_default();

                    let (selector, key) = split_selector_key(&path);
                    if key.is_empty() {
                        return (
                            "@avatar usage: @avatar.<name|description|owner|fragment|lang> [value] | @avatar.#<selector>.<name|description|owner|fragment|lang>".to_string(),
                            room_name.to_string(),
                        );
                    }

                    let caller_fragment = from_did
                        .fragment
                        .as_ref()
                        .map(|value| value.to_ascii_lowercase())
                        .unwrap_or_default();

                    let target_handle = if let Some(selector_token) = selector.as_ref() {
                        let rooms = self.rooms.read().await;
                        let Some(room) = rooms.get(room_name) else {
                            return (format!("@here room '{}' not found", room_name), room_name.to_string());
                        };
                        let mut selected: Option<String> = None;
                        for (handle, avatar) in room.avatars.iter() {
                            if handle.trim().eq_ignore_ascii_case(selector_token.as_str()) {
                                selected = Some(handle.clone());
                                break;
                            }
                            let avatar_fragment = avatar
                                .agent_did
                                .fragment
                                .as_ref()
                                .map(|value| value.to_ascii_lowercase())
                                .unwrap_or_default();
                            if !avatar_fragment.is_empty() && avatar_fragment == *selector_token {
                                selected = Some(handle.clone());
                                break;
                            }
                        }
                        let Some(found) = selected else {
                            return (
                                format!("@avatar selector '#{}' not found in room", selector_token),
                                room_name.to_string(),
                            );
                        };
                        found
                    } else {
                        from.to_string()
                    };

                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(room_name) else {
                        return (format!("@here room '{}' not found", room_name), room_name.to_string());
                    };
                    let Some(target_avatar) = room.avatars.get_mut(&target_handle) else {
                        return (format!("@avatar '{}' not found", target_handle), room_name.to_string());
                    };

                    let target_fragment = target_avatar
                        .agent_did
                        .fragment
                        .clone()
                        .unwrap_or_default();
                    let target_fragment_display = if target_fragment.trim().is_empty() {
                        "(none)".to_string()
                    } else {
                        format!("#{}", target_fragment)
                    };

                    if key == "_list" {
                        return (
                            format!(
                                "@ .avatar.name {}\n@ .avatar.description {}\n@ .avatar.owner {}\n@ .avatar.fragment {}\n@ .avatar.lang {}\n@ .avatar.acl {}\n@ .avatar.shortcuts {}",
                                target_avatar.inbox,
                                target_avatar.description_or_default(),
                                target_avatar.owner,
                                target_fragment_display,
                                target_avatar.language_order,
                                target_avatar.acl.summary(),
                                target_avatar.object_shortcuts_summary()
                            ),
                            room_name.to_string(),
                        );
                    }

                    if value.is_empty() {
                        return match key.as_str() {
                            "name" => (target_avatar.inbox.clone(), room_name.to_string()),
                            "description" => (target_avatar.description_or_default(), room_name.to_string()),
                            "owner" => (target_avatar.owner.clone(), room_name.to_string()),
                            "fragment" => (target_fragment_display, room_name.to_string()),
                            "lang" | "language" => (target_avatar.language_order.clone(), room_name.to_string()),
                            _ => (
                                format!("@avatar unknown attribute '{}'. Allowed: name, description, owner, fragment, lang", key),
                                room_name.to_string(),
                            ),
                        };
                    }

                    let caller_base = from_did.base_id();
                    let can_mutate = target_avatar.owner == caller_base;
                    if !can_mutate {
                        return (
                            "You don't have access to this.".to_string(),
                            room_name.to_string(),
                        );
                    }

                    match key.as_str() {
                        "description" => {
                            target_avatar.set_description(value.clone());
                            return (format!("@avatar.description {}", value), room_name.to_string());
                        }
                        "lang" | "language" => {
                            let Some(collapsed) = collapse_world_language_order_strict(&value) else {
                                return (
                                    format!(
                                        "@avatar language rejected. supported={}. Set language here, or leave.",
                                        supported_world_languages_text()
                                    ),
                                    room_name.to_string(),
                                );
                            };
                            target_avatar.language_order = collapsed.clone();
                            return (collapsed, room_name.to_string());
                        }
                        "name" => {
                            if selector.is_some() {
                                return (
                                    "@avatar.name update requires self target (@avatar.name <value>)".to_string(),
                                    room_name.to_string(),
                                );
                            }
                            let _ = caller_fragment;
                            return (
                                "@avatar.name is read-only in runtime; set alias/fragment at identity bootstrap.".to_string(),
                                room_name.to_string(),
                            );
                        }
                        "owner" | "fragment" => {
                            return (
                                format!("@avatar.{} is read-only", key),
                                room_name.to_string(),
                            );
                        }
                        _ => {
                            return (
                                format!("@avatar unknown attribute '{}'. Allowed: name, description, owner, fragment, lang", key),
                                room_name.to_string(),
                            );
                        }
                    }
                }

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

                        if !self.is_local_world_ipns(&object_did.ipns).await {
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
                        let world_ipns = self
                            .local_world_ipns()
                            .await
                            .unwrap_or_else(|| "unconfigured".to_string());
                        (object_id.clone(), create_world_did(&world_ipns, &object_id))
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
                    let fragment = from_did
                        .fragment
                        .clone()
                        .map(|value| format!("#{}", value))
                        .unwrap_or_else(|| "(none)".to_string());
                    return (format!(
                        "@ .avatar.name {}\n@ .avatar.description {}\n@ .avatar.owner {}\n@ .avatar.fragment {}\n@ .avatar.acl {}\n@ .avatar.shortcuts {}",
                        avatar.inbox,
                        avatar.description_or_default(),
                        avatar.owner,
                        fragment,
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
                                "@avatar language rejected. supported={}. Set language here, or leave.",
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

                // ── Avatar gameplay commands (look, go, inspect) ────────
                let caller_did = from_did.id();
                let avatar_ctx = {
                    let rooms = self.rooms.read().await;
                    let things = self.room_object_names(room_name).await;
                    rooms.get(room_name).map(|room| {
                        avatar_commands::AvatarCommandContext {
                            room_name: room_name.to_string(),
                            room_title: room.title_or_default(),
                            room_description: room.description_or_default(),
                            exits: room.exits.clone(),
                            avatars: room.avatars.keys().cloned().collect(),
                            things,
                            sender_profile: sender_profile.to_string(),
                            caller_did: caller_did.clone(),
                        }
                    })
                };

                if let Some(ref ctx) = avatar_ctx {
                    if let Some(result) = avatar_commands::execute_avatar_command(trimmed, ctx) {
                        match result.action {
                            avatar_commands::AvatarAction::None => {
                                return (result.response, room_name.to_string());
                            }
                            avatar_commands::AvatarAction::Move { exit } => {
                                return self
                                    .execute_avatar_move(room_name, from, &caller_did, sender_profile, &exit)
                                    .await;
                            }
                        }
                    }
                }

                // Unqualified input: fall through to room commands.
                (
                    self
                        .room_command(
                            room_name,
                            trimmed,
                            from,
                            sender_profile,
                            Some(caller_did.as_str()),
                        )
                        .await,
                    room_name.to_string(),
                )
            }
        }
    }

    async fn execute_avatar_move(
        &self,
        room_name: &str,
        from: &str,
        _caller_did: &str,
        sender_profile: &str,
        exit: &ma_core::ExitData,
    ) -> (String, String) {
        let prefs: Vec<String> = sender_profile
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let exit_name = exit.name_for_preferences(&prefs);
        let destination = exit.to.clone();
        let travel_text = exit.travel_text_for_preferences(&prefs);

        let (local_destination, external_destination) = match Did::try_from(destination.as_str()) {
            Ok(did) => {
                if self.is_local_world_ipns(&did.ipns).await {
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
        let Some(avatar) = avatar else {
            let base = travel_text
                .unwrap_or_else(|| format!("{} goes {}.", from, exit_name));
            return (base, room_name.to_string());
        };

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
            return (
                format!("Destination '{}' is not a room DID (missing fragment).", destination),
                room_name.to_string(),
            );
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
        (
            format!("Destination '{}' no longer exists.", local_destination),
            room_name.to_string(),
        )
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
                "@world commands: help | ping [room] | list | claim | broadcast <message> | knock list [all] | knock accept <id> | knock reject <id> [note] | knock delete <id> | invite <did> [note] | room <name> acl show|open|close|allow <did>|deny <did> | room <name> owner <did> | flush [knock|all] | migrate-index | save | load <cid> | dig <direction> [to|til <#dest|did>] | bury <direction>",
            );
        }

        let mut parts = normalized.splitn(2, char::is_whitespace);
        let method = parts.next().unwrap_or_default().to_ascii_lowercase();
        let arg = parts.next().unwrap_or_default().trim().to_string();

        if method == "broadcast" {
            if arg.is_empty() {
                return Reply::world("usage: @world.broadcast <message>").to_string();
            }
            return Reply::world(format!("broadcast sent to room '{}'", room_name)).to_string();
        }

        if let Some(property) = parse_property_command_for_keys(
            normalized,
            &["_list", "owner", "did", "rooms", "lang_cid"],
        ) {
            let path = property.key;
            let value = property.value.unwrap_or_default();

            if value.is_empty() {
                match path.as_str() {
                    "_list" => {
                        let owner = self
                            .owner_did
                            .read()
                            .await
                            .clone()
                            .unwrap_or_else(|| "(none)".to_string());
                        let did = self
                            .world_did
                            .read()
                            .await
                            .clone()
                            .unwrap_or_else(|| format!("{DID_PREFIX}unconfigured"));
                        let rooms = {
                            let rooms = self.rooms.read().await;
                            let mut names = rooms.keys().cloned().collect::<Vec<_>>();
                            names.sort();
                            if names.is_empty() {
                                "(none)".to_string()
                            } else {
                                names.join(",")
                            }
                        };
                        let lang_cid = self
                            .lang_cid
                            .read()
                            .await
                            .clone()
                            .unwrap_or_else(|| "(none)".to_string());
                        return Reply::attr_list(Scope::World, &[
                            ("owner", &owner),
                            ("did", &did),
                            ("rooms", &rooms),
                            ("lang_cid", &lang_cid),
                        ]);
                    }
                    "owner" => {
                        return self
                            .owner_did
                            .read()
                            .await
                            .clone()
                            .unwrap_or_else(|| "(none)".to_string())
                    }
                    "did" => {
                        return self
                            .world_did
                            .read()
                            .await
                            .clone()
                            .unwrap_or_else(|| format!("{DID_PREFIX}unconfigured"))
                    }
                    "rooms" => {
                        let rooms = self.rooms.read().await;
                        let mut names = rooms.keys().cloned().collect::<Vec<_>>();
                        names.sort();
                        if names.is_empty() {
                            return "(none)".to_string();
                        }
                        return names.join("\n");
                    }
                    "lang_cid" => {
                        return self
                            .lang_cid
                            .read()
                            .await
                            .clone()
                            .unwrap_or_else(|| "(none)".to_string())
                    }
                    _ => {
                        return Reply::world(format!(
                            "unknown attribute '{}'. Allowed: owner, did, rooms, lang_cid",
                            path
                        )).to_string()
                    }
                }
            }

            let owner_did = self.owner_did.read().await.clone();
            let is_owner = owner_did
                .as_ref()
                .map(|owner| owner == &from_did.base_id())
                .unwrap_or(false);

            match path.as_str() {
                "owner" => {
                    if owner_did.is_some() && !is_owner {
                        return "You don't have access to this.".to_string();
                    }
                    let normalized = value.trim();
                    return match self.set_owner_did(normalized).await {
                        Ok(root) => root,
                        Err(err) => Reply::world(format!("invalid owner DID '{}': {}", value, err)).to_string(),
                    };
                }
                "lang_cid" => {
                    if !is_owner {
                        return "You don't have access to this.".to_string();
                    }
                    self.set_lang_cid(Some(value.clone())).await;
                    return value;
                }
                "did" | "rooms" => {
                    return format!("@world.{} is read-only", path);
                }
                _ => {
                    return Reply::world(format!(
                        "unknown attribute '{}'. Allowed: owner, did, rooms, lang_cid",
                        path
                    )).to_string()
                }
            }
        }

        if !normalized.contains(char::is_whitespace) {
            let tree = self.public_inspect_tree().await;
            if let Some(value) = resolve_public_inspect_path(&tree, normalized) {
                return format_public_inspect_value(value);
            }
        }

        if let Some(property) = parse_property_command(normalized) {
            let path = property.key;
            let value = property.value.unwrap_or_default();
            if value.is_empty() && (path == "avatars" || path == "avatars._list" || path.starts_with("avatars.")) {
                let registry = self.avatar_registry.read().await;

                if path == "avatars" || path == "avatars._list" {
                    if registry.is_empty() {
                        return Reply::world("avatars: (none)").to_string();
                    }
                    let mut fragments = registry.keys().cloned().collect::<Vec<_>>();
                    fragments.sort();
                    return Reply::world(format!("avatars:\n{}", fragments.join("\n"))).to_string();
                }

                let mut parts = path.split('.');
                let root = parts.next().unwrap_or_default();
                let fragment = parts.next().unwrap_or_default().trim();
                let key = parts.collect::<Vec<_>>().join(".");
                if root != "avatars" || fragment.is_empty() {
                    return Reply::world("avatar path usage: @world.avatars.<fragment>.<field>").to_string();
                }
                let Some(entry) = registry.get(fragment) else {
                    return Reply::world(format!("avatar '{}' not found", fragment)).to_string();
                };

                if key.is_empty() || key == "_list" {
                    let p = format!("avatars.{}", fragment);
                    return Reply::join(&[
                        Reply::world_attr(format!("{p}.did"), &entry.did),
                        Reply::world_attr(format!("{p}.name"), &entry.name),
                        Reply::world_attr(format!("{p}.description"), &entry.description),
                        Reply::world_attr(format!("{p}.owner"), &entry.owner),
                        Reply::world_attr(format!("{p}.fragment"), &entry.fragment),
                        Reply::world_attr(format!("{p}.lang"), &entry.lang),
                        Reply::world_attr(format!("{p}.endpoint"), &entry.endpoint),
                        Reply::world_attr(format!("{p}.room"), &entry.room),
                        Reply::world_attr(format!("{p}.key_agreement"), &entry.key_agreement),
                        Reply::world_attr(format!("{p}.acl"), &entry.acl),
                        Reply::world_attr(format!("{p}.identity"), &entry.identity.cid),
                    ]);
                }

                return match key.as_str() {
                    "did" => entry.did.clone(),
                    "name" => entry.name.clone(),
                    "description" => entry.description.clone(),
                    "owner" => entry.owner.clone(),
                    "fragment" => entry.fragment.clone(),
                    "lang" | "language" => entry.lang.clone(),
                    "endpoint" => entry.endpoint.clone(),
                    "room" => entry.room.clone(),
                    "key_agreement" => entry.key_agreement.clone(),
                    "acl" => entry.acl.clone(),
                    "identity" | "doc" => entry.identity.cid.clone(),
                    "shortcuts" => {
                        if entry.shortcuts.is_empty() {
                            "(none)".to_string()
                        } else {
                            let mut rows = entry
                                .shortcuts
                                .iter()
                                .map(|(alias, object_id)| format!("{} -> {}", alias, object_id))
                                .collect::<Vec<_>>();
                            rows.sort();
                            rows.join("\n")
                        }
                    }
                    _ => Reply::world(format!(
                        "unknown avatar attribute '{}'. Allowed: did, name, description, owner, fragment, lang, endpoint, room, key_agreement, acl, shortcuts, identity",
                        key
                    )).to_string(),
                };
            }
        }

        // Command tokens are world/realm-defined and intentionally invariant.
        // Localized input aliases (e.g. "grave" -> "dig") belong in actor/client.

        if method == "list" {
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
            return Reply::world(format!("objects:\n{}", payload)).to_string();
        }

        // Caller's DID is directly available from from_did
        let caller_did = from_did.id();

        if method == "ping" {
            let room_hint = arg.trim().trim_start_matches('#');
            let effective_room = if !room_hint.is_empty() {
                let rooms = self.rooms.read().await;
                if rooms.contains_key(room_hint) {
                    room_hint.to_string()
                } else {
                    room_name.to_string()
                }
            } else {
                room_name.to_string()
            };
            let touched = self
                .touch_avatar_presence_for_did(&effective_room, &caller_did)
                .await;
            let room_did = self.build_room_did(&effective_room).await;
            return format!(
                "pong room_did={} touched={}",
                room_did,
                touched
            );
        }

        // @world.claim — set world owner to caller DID if unclaimed.
        if method == "claim" {
            let current_owner = self.owner_did.read().await.clone();
            let caller_base = from_did.base_id();
            if let Some(owner) = current_owner {
                if owner == caller_base {
                    return Reply::world(format!("already claimed by {}", owner)).to_string();
                }
                return Reply::world(format!("already claimed by {}", owner)).to_string();
            }

            {
                let mut owner = self.owner_did.write().await;
                *owner = Some(caller_base.clone());
            }
            self.allow_entry_did(&caller_base).await;
            info!("World claimed by {}", caller_base);
            return Reply::world(format!("claimed by {}", caller_base)).to_string();
        }

        // All remaining commands require world-owner privilege.
        // Escalate: accept the caller if they are the world owner directly, or
        // if they are an avatar whose registered owner DID is the world owner.
        let owner_did = self.owner_did.read().await.clone();
        let caller_base = from_did.base_id();
        let is_owner = if let Some(ref owner) = owner_did {
            owner == &caller_base
                || self
                    .avatar_registry
                    .read()
                    .await
                    .values()
                    .any(|entry| entry.did == caller_did && entry.owner == *owner)
        } else {
            false
        };

        if !is_owner {
            return tr_world(
                active_lang,
                "world.owner.required",
                "@world only the world owner can run that command.",
            );
        }

        if method == "knock" {
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
                return Reply::world(format!("knock inbox:\n{}", lines.join("\n"))).to_string();
            }

            if sub == "accept" {
                let Some(id_raw) = parts.next() else {
                    return Reply::world("usage: @world.knock accept <id>").to_string();
                };
                let id = match Self::parse_knock_id_arg(id_raw) {
                    Ok(value) => value,
                    Err(err) => return Reply::world(format!("{}", err)).to_string(),
                };
                return match self.accept_knock(id).await {
                    Ok(item) => Reply::world(format!(
                        "knock accepted id={} did={} room={} (entry allowlist updated)",
                        item.id, item.requester_did, item.room
                    )).to_string(),
                    Err(err) => Reply::world(format!("knock accept failed: {}", err)).to_string(),
                };
            }

            if sub == "reject" {
                let Some(id_raw) = parts.next() else {
                    return Reply::world("usage: @world.knock reject <id> [note]").to_string();
                };
                let id = match Self::parse_knock_id_arg(id_raw) {
                    Ok(value) => value,
                    Err(err) => return Reply::world(format!("{}", err)).to_string(),
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
                    Ok(item) => Reply::world(format!(
                        "knock rejected id={} did={} room={}",
                        item.id, item.requester_did, item.room
                    )).to_string(),
                    Err(err) => Reply::world(format!("knock reject failed: {}", err)).to_string(),
                };
            }

            if sub == "delete" {
                let Some(id_raw) = parts.next() else {
                    return Reply::world("usage: @world.knock delete <id>").to_string();
                };
                let id = match id_raw.parse::<u64>() {
                    Ok(value) => value,
                    Err(_) => return Reply::world(format!("invalid knock id '{}'", id_raw)).to_string(),
                };
                return match self.delete_knock(id).await {
                    Ok(()) => Reply::world(format!("knock deleted id={}", id)).to_string(),
                    Err(err) => Reply::world(format!("knock delete failed: {}", err)).to_string(),
                };
            }

            return Reply::world("usage: @world.knock list [all] | @world.knock accept <id> | @world.knock reject <id> [note] | @world.knock delete <id>")
                .to_string();
        }

        if method == "invite" {
            let mut parts = arg.split_whitespace();
            let Some(target_did_raw) = parts.next() else {
                return Reply::world("usage: @world.invite <did> [note]").to_string();
            };

            let target_did = match Self::parse_invite_did_arg(target_did_raw) {
                Ok(root) => root,
                Err(err) => return Reply::world(format!("{}", err)).to_string(),
            };

            let invite_note = {
                let rest = parts.collect::<Vec<_>>().join(" ");
                if rest.trim().is_empty() {
                    "Your knock request was accepted. You may enter now.".to_string()
                } else {
                    rest
                }
            };

            self.allow_entry_did(&target_did).await;
            return Reply::world(format!(
                "invited {} (allowlisted). note='{}'",
                target_did,
                invite_note
            )).to_string();
        }

        if method == "flush" {
            let scope = arg.trim().to_ascii_lowercase();
            if scope.is_empty() || scope == "all" {
                let knocks = self.flush_knock_inbox().await;
                return Reply::world(format!("flush all: knocks={}", knocks)).to_string();
            }
            if scope == "knock" || scope == "knocks" {
                let removed = self.flush_knock_inbox().await;
                return Reply::world(format!("flush knock: removed={}", removed)).to_string();
            }
            return Reply::world("usage: @world.flush [knock|all]").to_string();
        }

        if method == "migrate-index" {
            let room_names = {
                let rooms = self.rooms.read().await;
                let mut names = rooms.keys().cloned().collect::<Vec<_>>();
                names.sort();
                names
            };

            if room_names.is_empty() {
                return Reply::world("migrate-index: no rooms to persist").to_string();
            }

            match self.save_rooms_and_world_index(&room_names).await {
                Ok(new_cid) => {
                    return Reply::world(format!(
                        "migrate-index complete: root_cid={} rooms={}",
                        new_cid,
                        room_names.len()
                    )).to_string();
                }
                Err(e) => {
                    return Reply::world(format!("migrate-index failed: {}", e)).to_string();
                }
            }
        }

        if method == "publish" {
            match self.publish_to_ipns().await {
                Ok(()) => {
                    return Reply::world("published to IPNS").to_string();
                }
                Err(e) => {
                    return Reply::world(format!("publish failed: {}", e)).to_string();
                }
            }
        }

        if method == "save" {
            match self.save_and_publish().await {
                Ok((state_cid, root_cid)) => {
                    return Reply::world(format!(
                        "saved and published: state_cid={} root_cid={}",
                        state_cid, root_cid
                    )).to_string();
                }
                Err(e) => {
                    return Reply::world(format!("save failed: {}", e)).to_string();
                }
            }
        }

        if method == "load" {
            if arg.is_empty() {
                return Reply::world("usage: @world.load <cid>").to_string();
            }
            match self.load_encrypted_state(arg.as_str()).await {
                Ok(root_cid) => {
                    return Reply::world(format!(
                        "loaded encrypted runtime state from {} (root_cid={})",
                        arg, root_cid
                    )).to_string();
                }
                Err(e) => {
                    return Reply::world(format!("load failed: {}", e)).to_string();
                }
            }
        }

        if method == "dig" {
            if arg.is_empty() {
                return Reply::world("usage: @world.dig <direction> [to|til <#dest|did:ma:...#room>]").to_string();
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
                    if self.is_local_world_ipns(&did.ipns).await {
                        let Some(fragment) = did.fragment.clone() else {
                            return Reply::world("usage: @world.dig <direction> [to <#dest|did:ma:...#room>]").to_string();
                        };
                        exit_target = fragment.clone();
                        local_room_to_create = Some(fragment);
                    } else {
                        exit_target = did.id();
                    }
                }
                Err(e) => {
                    if destination_input.contains(':') {
                        return Reply::world(format!("invalid destination DID '{}': {}", destination_input, e)).to_string();
                    }
                    let local_id = normalize_local_object_id(&destination_input);
                    if !is_valid_nanoid_id(&local_id) {
                        return Reply::world(format!(
                            "invalid destination id '{}': expected nanoid-compatible id ([A-Za-z0-9_-]+)",
                            destination_input
                        )).to_string();
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
            return Reply::world(format!("exit '{}' dug from '{}' → '{}'", exit_name, room_name, exit_target)).to_string();
        }

        if method == "room" {
            // @world.room <name> acl show|open|close|allow <did>|deny <did>
            // @world.room <name> owner <did:ma:...#fragment>
            // World-owner admin override for room-level ACLs.
            // Does NOT automatically bypass the ACL — caller must change it explicitly.
            let mut room_parts = arg.splitn(3, char::is_whitespace);
            let room_name_arg = room_parts.next().unwrap_or_default().trim().to_string();
            let sub = room_parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let sub_arg = room_parts.next().unwrap_or_default().trim().to_string();

            if room_name_arg.is_empty() || sub.is_empty() {
                return Reply::world("usage: @world.room <name> acl show|open|close|allow <did>|deny <did> | @world.room <name> owner <did:ma:...#fragment>").to_string();
            }

            if sub == "owner" {
                if sub_arg.is_empty() {
                    return Reply::world(format!(
                        "usage: @world.room {} owner <did:ma:...#fragment>",
                        room_name_arg
                    )).to_string();
                }
                if !sub_arg.contains('#') {
                    return Reply::world(format!(
                        "invalid owner DID '{}': missing #fragment",
                        sub_arg
                    )).to_string();
                }
                let target_did = match Did::try_from(sub_arg.as_str()) {
                    Ok(d) => {
                        if d.fragment.is_none() {
                            return Reply::world(format!(
                                "invalid owner DID '{}': missing #fragment",
                                sub_arg
                            )).to_string();
                        }
                        d.id()
                    }
                    Err(e) => return Reply::world(format!("invalid owner DID '{}': {}", sub_arg, e)).to_string(),
                };

                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&room_name_arg) else {
                    return Reply::world(format!("room '{}' not found", room_name_arg)).to_string();
                };
                room.acl.owner = Some(target_did.clone());
                room.acl.allow.insert(target_did.clone());
                room.acl.deny.remove(&target_did);
                drop(rooms);
                let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                return Reply::world(format!("room '{}' owner set to {}", room_name_arg, target_did)).to_string();
            }

            if sub != "acl" {
                return Reply::world("usage: @world.room <name> acl show|open|close|allow <did>|deny <did> | @world.room <name> owner <did:ma:...#fragment>").to_string();
            }

            let mut acl_parts = sub_arg.splitn(2, char::is_whitespace);
            let acl_cmd = acl_parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let acl_arg = acl_parts.next().unwrap_or_default().trim().to_string();

            match acl_cmd.as_str() {
                "" | "show" => {
                    let rooms = self.rooms.read().await;
                    let Some(room) = rooms.get(&room_name_arg) else {
                        return Reply::world(format!("room '{}' not found", room_name_arg)).to_string();
                    };
                    return Reply::world(format!(
                        "room '{}' acl: {} owner={}",
                        room_name_arg,
                        room.acl.summary(),
                        room.acl.owner.as_deref().unwrap_or("(none)")
                    )).to_string();
                }
                "open" => {
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return Reply::world(format!("room '{}' not found", room_name_arg)).to_string();
                    };
                    room.acl.allow.insert("*".to_string());
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return Reply::world(format!("room '{}' acl opened (public)", room_name_arg)).to_string();
                }
                "close" => {
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return Reply::world(format!("room '{}' not found", room_name_arg)).to_string();
                    };
                    room.acl.allow.remove("*");
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return Reply::world(format!("room '{}' acl closed (private)", room_name_arg)).to_string();
                }
                "allow" => {
                    if acl_arg.is_empty() {
                        return Reply::world(format!("usage: @world.room {} acl allow <did>", room_name_arg)).to_string();
                    }
                    let target_did = match Did::try_from(acl_arg.as_str()) {
                        Ok(d) => d.id(),
                        Err(e) => return Reply::world(format!("invalid DID '{}': {}", acl_arg, e)).to_string(),
                    };
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return Reply::world(format!("room '{}' not found", room_name_arg)).to_string();
                    };
                    room.acl.allow.insert(target_did.clone());
                    room.acl.deny.remove(&target_did);
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return Reply::world(format!("room '{}' acl: allowed {}", room_name_arg, target_did)).to_string();
                }
                "deny" => {
                    if acl_arg.is_empty() {
                        return Reply::world(format!("usage: @world.room {} acl deny <did>", room_name_arg)).to_string();
                    }
                    let target_did = match Did::try_from(acl_arg.as_str()) {
                        Ok(d) => d.id(),
                        Err(e) => return Reply::world(format!("invalid DID '{}': {}", acl_arg, e)).to_string(),
                    };
                    let mut rooms = self.rooms.write().await;
                    let Some(room) = rooms.get_mut(&room_name_arg) else {
                        return Reply::world(format!("room '{}' not found", room_name_arg)).to_string();
                    };
                    if room.acl.owner.as_deref() == Some(target_did.as_str()) {
                        return Reply::world(format!("room '{}' owner cannot be denied", room_name_arg)).to_string();
                    }
                    room.acl.deny.insert(target_did.clone());
                    room.acl.allow.remove(&target_did);
                    drop(rooms);
                    let _ = self.save_rooms_and_world_index(&[room_name_arg.clone()]).await;
                    return Reply::world(format!("room '{}' acl: denied {}", room_name_arg, target_did)).to_string();
                }
                _ => {
                    return Reply::world(format!(
                        "unknown acl subcommand '{}'. usage: @world.room {} acl show|open|close|allow <did>|deny <did>",
                        acl_cmd, room_name_arg
                    )).to_string();
                }
            }
        }

        Reply::world(format!("unknown command: {}", normalized)).to_string()
    }

    async fn room_command(
        &self,
        room_name: &str,
        command: &str,
        from: &str,
        _sender_profile: &str,
        caller_did: Option<&str>,
    ) -> String {

        let (room_exists, avatars, acl_owner, acl_summary, caller_owner, title, description, did) = {
            let rooms = self.rooms.read().await;
            if let Some(room) = rooms.get(room_name) {
                (
                    true,
                    room.avatars.iter()
                        .map(|(handle, avatar)| (handle.clone(), avatar.agent_did.id()))
                        .collect::<Vec<_>>(),
                    room.acl.owner.clone(),
                    room.acl.summary(),
                    room.avatars.get(from).map(|avatar| avatar.owner.clone()),
                    room.title_or_default(),
                    room.description_or_default(),
                    Some(room.did.clone()),
                )
            } else {
                (false, Vec::new(), None, "(none)".to_string(), None, String::new(), String::new(), None)
            }
        };
        let things = self.room_object_names(room_name).await;

        let ctx = RoomActorContext {
            room_name,
            room_exists,
            avatars,
            things,
            acl_owner: acl_owner.as_deref(),
            acl_summary: &acl_summary,
            caller_did,
            caller_owner: caller_owner.as_deref(),
            title: &title,
            description: &description,
            did: did.as_deref(),
        };

        let trimmed = command.trim();
        if trimmed.eq_ignore_ascii_case("ping") || trimmed.eq_ignore_ascii_case("ping?") {
            let who = caller_did.unwrap_or(from);
            return format!("@here pong room={} by={}", room_name, who);
        }

        // say: broadcast speech to everyone in the room.
        if let Some(rest) = trimmed.strip_prefix("say") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                let speech = normalize_spoken_text(rest.trim());
                if speech.is_empty() {
                    return "@here say what?".to_string();
                }
                info!("[{}] {}: {}", room_name, from, speech);
                self.record_event(format!("[{room_name}] {from}: {speech}")).await;
                self.record_room_event(
                    room_name,
                    "speech",
                    Some(from.to_string()),
                    caller_did.map(|d| d.to_string()),
                    None,
                    speech.clone(),
                )
                .await;
                return format!("{}: {}", from, speech);
            }
        }

        // emote: broadcast an emote action to everyone in the room.
        if let Some(rest) = trimmed.strip_prefix("emote") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                let action = normalize_spoken_text(rest.trim());
                if action.is_empty() {
                    return "@here emote what?".to_string();
                }
                info!("[{}] {} {}", room_name, from, action);
                self.record_event(format!("[{room_name}] * {from} {action}")).await;
                self.record_room_event(
                    room_name,
                    "emote",
                    Some(from.to_string()),
                    caller_did.map(|d| d.to_string()),
                    None,
                    action.clone(),
                )
                .await;
                return format!("* {} {}", from, action);
            }
        }

        if let Some(rest) = trimmed.strip_prefix("l ") {
            let did_raw = rest.trim();
            if did_raw.is_empty() {
                return "@here usage: l <did:ma:...[#fragment]>".to_string();
            }
            let did_query = match Did::try_from(did_raw) {
                Ok(did) => did,
                Err(err) => return format!("@here invalid DID '{}': {}", did_raw, err),
            };

            if let Some((_room, _handle, _did, _endpoint, description)) =
                self.find_avatar_presence_by_did(&did_query).await
            {
                return description;
            }
            if let Some(description) = self.did_description_fallback(&did_query).await {
                return description;
            }
            return format!("@here no avatar found for {}", did_query.id());
        }

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
                if let Some(root) = caller_did {
                    return format!("did={} source=caller handle={}", root, from);
                }
                return "@here caller DID unavailable".to_string();
            }
            if let Some(object_id) = self.resolve_room_object_id(room_name, token).await {
                let world_ipns = self
                    .local_world_ipns()
                    .await
                    .unwrap_or_else(|| "unconfigured".to_string());
                return format!(
                    "did={} source=object room={} object_id={} token={}",
                    create_world_did(&world_ipns, &object_id),
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
                        avatar.agent_did.id(),
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
                        room.avatars.retain(|_, av| av.agent_did.id() != did);
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
                        let normalized = value.trim();
                        let did = match Did::try_from(normalized) {
                            Ok(d) => d.id(),
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
                                    warn!("Failed to save world index after @here.content-b64: {}", e);
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
                                response = "@here usage: @here.exit-content-b64 <exit-id> <base64-yaml>".to_string();
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
                                response = "@here room has no published CID yet; use @here.content-b64 for full room YAML first".to_string();
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
                        if self.is_local_world_ipns(&did.ipns).await {
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
                        if let Some(caller) = caller_did {
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
            RoomActorAction::Bury { exit_name } => {
                let mut rooms = self.rooms.write().await;
                if let Some(room) = rooms.get_mut(room_name) {
                    let before = room.exits.len();
                    room.exits.retain(|exit| !exit.matches(&exit_name));
                    if room.exits.len() == before {
                        response = format!("@here exit '{}' not found in '{}'", exit_name, room_name);
                    } else {
                        changed_rooms.push(room_name.to_string());
                        room_update_announcement = Some(format!("exit '{}' buried by {}", exit_name, from));
                    }
                }
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
                caller_did.map(|v| v.to_string()),
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
        let room_did_parsed = Did::try_from(room_did)
            .map_err(|e| anyhow!("invalid room did '{}': {}", room_did, e))?;
        let room_did_canonical = room_did_parsed.id();
        let signing_did = Did::new_root(&room_did_parsed.ipns)
            .map_err(|e| anyhow!("invalid signing did for room {}: {}", room_did, e))?;

        if let Some(room_key) = {
            let slots = self.world.actor_secrets.read().await;
            slots
                .get(room_did)
                .or_else(|| slots.get(room_did_canonical.as_str()))
                .map(|secret| secret.signing_key)
        } {
            return SigningKey::from_private_key_bytes(signing_did.clone(), room_key)
                .map_err(|e| anyhow!("failed to restore signing key for room {}: {}", room_did, e));
        }

        if let Some(world_key) = {
            let world_key_guard = self.world.unlocked_world_signing_key.read().await;
            *world_key_guard
        } {
            return SigningKey::from_private_key_bytes(signing_did, world_key)
                .map_err(|e| anyhow!("failed to restore fallback signing key for room {}: {}", room_did, e));
        }

        Err(anyhow!(
            "missing signing key for room {}: missing room actor secret and missing unlocked world signing key",
            room_did
        ))
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

    fn push_cache_key(target_endpoint_id: &str, lane_alpn: &'static [u8]) -> String {
        format!("{}|{}", String::from_utf8_lossy(lane_alpn), target_endpoint_id.trim())
    }

    async fn create_push_stream_cache(
        &self,
        target_endpoint_id: &str,
        lane_alpn: &'static [u8],
    ) -> Result<PushStreamCache> {
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

        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|e| anyhow!("push connection.open_bi failed: {}", e))?;

        Ok(PushStreamCache {
            connection,
            send,
            recv,
        })
    }

    async fn exchange_push_on_stream(
        &self,
        cache: &mut PushStreamCache,
        message_cbor: Vec<u8>,
    ) -> Result<()> {
        let request = OutboxRequest::Signed { message_cbor };
        let payload = serde_json::to_vec(&request)
            .map_err(|e| anyhow!("failed to serialize outbox request: {}", e))?;

        cache
            .send
            .write_u32(payload.len() as u32)
            .await
            .map_err(|e| anyhow!("push write_u32 failed: {}", e))?;
        cache
            .send
            .write_all(&payload)
            .await
            .map_err(|e| anyhow!("push write_all failed: {}", e))?;
        cache
            .send
            .flush()
            .await
            .map_err(|e| anyhow!("push flush failed: {}", e))?;

        let frame_len = cache
            .recv
            .read_u32()
            .await
            .map_err(|e| anyhow!("push read_u32 failed: {}", e))? as usize;
        if frame_len > 256 * 1024 {
            return Err(anyhow!("push response frame too large: {}", frame_len));
        }

        let mut bytes = vec![0u8; frame_len];
        cache
            .recv
            .read_exact(&mut bytes)
            .await
            .map_err(|e| anyhow!("push read_exact failed: {}", e))?;
        let response: OutboxResponse = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow!("push response decode failed: {}", e))?;
        if !response.ok {
            return Err(anyhow!("push rejected: {}", response.message));
        }

        Ok(())
    }

    async fn send_signed_push_to_endpoint_on_lane(
        &self,
        target_endpoint_id: &str,
        message_cbor: Vec<u8>,
        lane_alpn: &'static [u8],
    ) -> Result<()> {
        let cache_key = Self::push_cache_key(target_endpoint_id, lane_alpn);
        let now = Instant::now();
        {
            let mut cooldowns = self.push_timeout_cooldown.lock().await;
            cooldowns.retain(|_, until| *until > now);
            if let Some(until) = cooldowns.get(&cache_key) {
                debug!(
                    "push endpoint {} on lane {} is in cooldown for {:?}",
                    target_endpoint_id,
                    String::from_utf8_lossy(lane_alpn),
                    until.saturating_duration_since(now)
                );
                return Ok(());
            }
        }
        let mut last_error: Option<anyhow::Error> = None;

        for _ in 0..2 {
            let mut stream_cache = {
                let mut slots = self.push_stream_cache.lock().await;
                slots.remove(&cache_key)
            }
            .unwrap_or(self.create_push_stream_cache(target_endpoint_id, lane_alpn).await?);

            match self
                .exchange_push_on_stream(&mut stream_cache, message_cbor.clone())
                .await
            {
                Ok(()) => {
                    let mut slots = self.push_stream_cache.lock().await;
                    slots.insert(cache_key.clone(), stream_cache);
                    return Ok(());
                }
                Err(err) => {
                    if err.to_string().contains("timed out") {
                        let until = Instant::now() + Duration::from_secs(PUSH_TIMEOUT_COOLDOWN_SECS);
                        let mut cooldowns = self.push_timeout_cooldown.lock().await;
                        cooldowns.insert(cache_key.clone(), until);
                        warn!(
                            "push timeout for endpoint {} on lane {}; cooling down for {}s",
                            target_endpoint_id,
                            String::from_utf8_lossy(lane_alpn),
                            PUSH_TIMEOUT_COOLDOWN_SECS
                        );
                        stream_cache.connection.close(0u32.into(), b"push timeout");
                        return Ok(());
                    }
                    last_error = Some(err);
                    stream_cache.connection.close(0u32.into(), b"stream error");
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("push failed")))
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

    async fn push_presence_room_state_to(
        &self,
        room_name: &str,
        target_endpoint_id: &str,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_did, room_title, room_description, avatars, _endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence room-state context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };

        let signing_key = match self.room_signing_key(&room_did).await {
            Ok(key) => key,
            Err(err) => {
                warn!("presence room-state signing key unavailable for {}: {}", room_did, err);
                return;
            }
        };

        let latest_event_sequence = self
            .world
            .latest_room_event_sequence(room_name)
            .await
            .unwrap_or(0);
        let room_object_dids = self.world.room_object_did_map(room_name).await;

        let payload = PresenceRoomStateEvent {
            v: 1,
            kind: "presence.room_state".to_string(),
            room: room_name.to_string(),
            room_did: room_did.clone(),
            room_title,
            room_description,
            avatars,
            latest_event_sequence,
            room_object_dids,
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence room-state encode failed for room '{}': {}", room_name, err);
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
                warn!("presence room-state message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence room-state cbor encode failed: {}", err);
                return;
            }
        };

        if let Err(err) = self.send_signed_push_to_endpoint(target_endpoint_id, cbor).await {
            warn!("presence room-state push to {} failed: {}", target_endpoint_id, err);
        }
    }

    async fn push_room_events(&self, room_name: &str, since_sequence: u64) {
        let context = self.room_presence_context(room_name).await;
        let (room_did, room_title, room_description, avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("room events context unavailable for '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_did).await {
            Ok(key) => key,
            Err(err) => {
                warn!("room event signing key unavailable for {}: {}", room_did, err);
                return;
            }
        };
        let (events, latest_event_sequence) = match self.world.room_events_since(room_name, since_sequence).await {
            Ok(val) => val,
            Err(err) => {
                warn!("room events unavailable for '{}': {}", room_name, err);
                return;
            }
        };
        if events.is_empty() {
            return;
        }

        for event in events {
            let payload = RoomEventEnvelope {
                v: 1,
                kind: "room.event".to_string(),
                room: room_name.to_string(),
                room_did: room_did.clone(),
                room_title: room_title.clone(),
                room_description: room_description.clone(),
                avatars: avatars.clone(),
                event,
                latest_event_sequence,
                ts: Utc::now().to_rfc3339(),
            };
            let content = match serde_json::to_vec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("room event encode failed for '{}': {}", room_name, err);
                    continue;
                }
            };
            let message = match Message::new(
                room_did.clone(),
                room_did.clone(),
                CONTENT_TYPE_EVENT,
                content,
                &signing_key,
            ) {
                Ok(msg) => msg,
                Err(err) => {
                    warn!("room event message build failed: {}", err);
                    continue;
                }
            };
            let cbor = match message.to_cbor() {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("room event cbor encode failed: {}", err);
                    continue;
                }
            };

            for endpoint in &endpoints {
                let cbor_clone = cbor.clone();
                if let Err(err) = self
                    .send_signed_push_to_endpoint_on_lane(endpoint, cbor_clone, PRESENCE_ALPN)
                    .await
                {
                    warn!("room event push to {} failed: {}", endpoint, err);
                }
            }
        }
    }

    async fn push_world_broadcast(&self, room_name: &str, message_text: &str) {
        let text = message_text.trim();
        if text.is_empty() {
            return;
        }

        let context = self.room_presence_context(room_name).await;
        let (room_did, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("world broadcast context unavailable for '{}': {}", room_name, err);
                return;
            }
        };
        if endpoints.is_empty() {
            return;
        }

        let signing_key = match self.room_signing_key(&room_did).await {
            Ok(key) => key,
            Err(err) => {
                warn!("world broadcast signing key unavailable for {}: {}", room_did, err);
                return;
            }
        };

        let payload = WorldBroadcastEnvelope {
            v: 1,
            kind: "world.broadcast".to_string(),
            room: room_name.to_string(),
            room_did: room_did.clone(),
            message: text.to_string(),
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("world broadcast encode failed for '{}': {}", room_name, err);
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
                warn!("world broadcast message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("world broadcast cbor encode failed: {}", err);
                return;
            }
        };

        for endpoint in endpoints {
            let cbor_clone = cbor.clone();
            if let Err(err) = self
                .send_signed_push_to_endpoint_on_lane(&endpoint, cbor_clone, INBOX_ALPN)
                .await
            {
                warn!("world broadcast push to {} failed: {}", endpoint, err);
            }
        }
    }

    async fn push_presence_refresh_request_to(
        &self,
        room_name: &str,
        target_endpoint_id: &str,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_did, _room_title, _room_description, _avatars, _endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence refresh context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_did).await {
            Ok(key) => key,
            Err(err) => {
                warn!("presence refresh signing key unavailable for {}: {}", room_did, err);
                return;
            }
        };

        let payload = PresenceRefreshRequestEvent {
            v: 1,
            kind: "presence.refresh.request".to_string(),
            room: room_name.to_string(),
            room_did: room_did.clone(),
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence refresh encode failed for room '{}': {}", room_name, err);
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
                warn!("presence refresh message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence refresh cbor encode failed: {}", err);
                return;
            }
        };

        if let Err(err) = self.send_signed_push_to_endpoint(target_endpoint_id, cbor).await {
            warn!("presence refresh push to {} failed: {}", target_endpoint_id, err);
        }
    }

    async fn push_presence_refresh_request(&self, room_name: &str) {
        let context = self.room_presence_context(room_name).await;
        let (_room_did, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence refresh context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };

        for endpoint in endpoints {
            self.push_presence_refresh_request_to(room_name, &endpoint).await;
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
                    world_did: String::new(),
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
        let cache_key = sender_root.ipns.clone();

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
        let cache_key = sender_root.ipns.clone();
        let mut cache = self.did_cache.write().await;
        if let Some(cached) = cache.get_mut(&cache_key) {
            cached.dirty = dirty;
        }
    }

    async fn verify_message(&self, message_cbor: Vec<u8>) -> Result<(Message, Did, Document)> {
        let message = Message::from_cbor(&message_cbor)?;
        let sender_did = Did::try_from(message.from.as_str())?;
        let as_onboarding_did_error = |source: &anyhow::Error| {
            let detail = source.to_string();
            let lowered = detail.to_ascii_lowercase();
            if lowered.contains("failed to fetch did document")
                || lowered.contains("name/resolve failed")
                || lowered.contains("/ipns/")
                || lowered.contains("did document") && lowered.contains("not found")
            {
                anyhow!(
                    "did document is not published yet for {} (submit document via ma/ipfs/1): {}",
                    sender_did.id(),
                    detail
                )
            } else {
                anyhow!(detail)
            }
        };

        let t0 = std::time::Instant::now();
        let (sender_document, fetched_from_kubo, is_dirty) = self.get_sender_document(&sender_did, false).await
            .map_err(|e| {
                warn!("DID doc fetch failed for {} after {:?}: {}", sender_did.id(), t0.elapsed(), e);
                as_onboarding_did_error(&e)
            })?;
        if fetched_from_kubo {
            info!("DID doc for {} fetched via Kubo in {:?}", sender_did.id(), t0.elapsed());
        } else {
            debug!("DID doc for {} served from cache in {:?}", sender_did.id(), t0.elapsed());
        }
        if is_dirty {
            warn!("DID {} is marked dirty; using cached document", sender_did.id());
        }

        if message.verify_with_document(&sender_document).is_ok() {
            if is_dirty {
                self.set_sender_dirty(&sender_did, false).await;
                info!("DID {} cleared from dirty state after successful verification", sender_did.id());
            }
            return Ok((message, sender_did, sender_document));
        }

        warn!(
            "signature verification failed with cached DID doc for {}; retrying fresh fetch",
            sender_did.id()
        );

        let refresh_t0 = std::time::Instant::now();
        let (refreshed_document, refreshed_from_kubo, _) =
            match self.get_sender_document(&sender_did, true).await {
                Ok(value) => value,
                Err(e) => {
                    self.set_sender_dirty(&sender_did, true).await;
                    warn!(
                        "forced DID doc refetch failed for {} after {:?}: {}",
                        sender_did.id(),
                        refresh_t0.elapsed(),
                        e
                    );
                    return Err(as_onboarding_did_error(&e));
                }
            };
        if refreshed_from_kubo {
            info!(
                "DID doc for {} force-fetched via Kubo in {:?}",
                sender_did.id(),
                refresh_t0.elapsed()
            );
        }

        if message.verify_with_document(&refreshed_document).is_ok() {
            self.set_sender_dirty(&sender_did, false).await;
            return Ok((message, sender_did, refreshed_document));
        }

        self.set_sender_dirty(&sender_did, true).await;
        warn!(
            "DID {} marked dirty: signature verification still failed after forced refresh",
            sender_did.id()
        );

        Err(anyhow!(
            "message signature verification failed for {} (cached and refreshed DID document)",
            sender_did.id()
        ))
    }

    async fn handle_request(&self, request: WorldRequest, agent_endpoint: String) -> Result<WorldResponse> {
        if !self.lane.supports_request(&request) {
            return Err(anyhow!(
                "lane '{}' does not support this request type",
                self.lane.label()
            ));
        }

        // Each ALPN lane has exactly one canonical content type.
        let (message, sender_did, sender_document) = self.verify_message(request.message_cbor).await?;
        let expected_ct = CONTENT_TYPE_WORLD;
        if !Self::content_type_matches(&message.content_type, expected_ct, "application/x-ma-command") {
            return Err(anyhow!("expected {} on this lane, got {}", expected_ct, message.content_type));
        }
        let command: WorldCommand = serde_json::from_slice(&message.content)
            .map_err(|err| anyhow!("invalid command payload: {}", err))?;
        if let Some(method) = command.internal_method() {
            debug!("processing internal method '{}' on lane '{}'", method, self.lane.label());
        }
        self.handle_command(command, &message, &sender_did, sender_document, agent_endpoint).await
    }

    async fn handle_command(
        &self,
        command: WorldCommand,
        message: &Message,
        sender_did: &Did,
        sender_document: Document,
        agent_endpoint: String,
    ) -> Result<WorldResponse> {
        let sender_profile = sender_profile_from_document(&sender_document);
        let sender_push_endpoint = sender_push_endpoint_from_document(&sender_document)
            .unwrap_or_else(|| agent_endpoint.clone());
        let sender_encryption_pubkey_multibase =
            sender_encryption_pubkey_multibase_from_document(&sender_document)?;
        if sender_push_endpoint != agent_endpoint {
            debug!(
                "sender endpoint drift: request_remote={} did_push_endpoint={} did={}",
                agent_endpoint,
                sender_push_endpoint,
                sender_did.id()
            );
        }

        match command {
            WorldCommand::Ping { room_did } => {
                let pinged_room = if room_did.is_empty() {
                    DEFAULT_ROOM.to_string()
                } else if let Ok(did) = Did::try_from(room_did.as_str()) {
                    did.fragment.clone().unwrap_or_else(|| DEFAULT_ROOM.to_string())
                } else {
                    room_did.clone() // Postel: accept plain room name
                };

                // Derive the avatar DID so we can look up existing location.
                let world_ipns = self.world
                    .local_world_ipns()
                    .await
                    .unwrap_or_else(|| "unconfigured".to_string());
                let avatar_fragment = sender_did
                    .fragment
                    .clone()
                    .unwrap_or_else(|| "avatar".to_string())
                    .trim()
                    .trim_start_matches('@')
                    .to_string();
                let avatar_did_str = create_world_did(&world_ipns, &avatar_fragment);

                // Check if avatar already exists somewhere.
                let existing_room = self.world.avatar_room_for_did(&avatar_did_str).await;

                let (actual_room, handle, created) = if let Some(room) = existing_room {
                    // Avatar exists — report actual location, touch presence, don't move.
                    let _ = self.world
                        .touch_avatar_presence_for_did(&room, &avatar_did_str)
                        .await;
                    self.world
                        .upsert_avatar_location(&room, &avatar_did_str, &sender_push_endpoint)
                        .await;
                    let h = self.world
                        .avatar_handle_for_did(&room, &avatar_did_str)
                        .await
                        .unwrap_or_else(|| avatar_fragment.clone());
                    (room, h, false)
                } else {
                    // First contact — ensure avatar in the pinged room.
                    let (_, h, c) = self.world.ensure_avatar(
                        sender_did,
                        &sender_profile,
                        &sender_push_endpoint,
                        &sender_encryption_pubkey_multibase,
                        &pinged_room,
                    ).await?;
                    (pinged_room.clone(), h, c)
                };

                if created {
                    self.push_presence_snapshot(&actual_room).await;
                }

                let actual_room_did = self.world.room_did(&actual_room).await;
                Ok(WorldResponse {
                    ok: true,
                    room: actual_room.clone(),
                    message: format!("pong {}", actual_room_did),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence: self.world.latest_room_event_sequence(&actual_room).await.unwrap_or(0),
                    broadcasted: false,
                    events: Vec::new(),
                    handle,
                    room_description: self.world.room_description(&actual_room).await,
                    room_title: self.world.room_title(&actual_room).await,
                    room_did: actual_room_did,
                    world_did: self.world.world_did.read().await.clone().unwrap_or_default(),
                    avatars: self.world.room_avatars(&actual_room).await,
                    room_object_dids: self.world.room_object_did_map(&actual_room).await,
                    transport_ack: None,
                })
            }
            WorldCommand::Message { room, envelope } => {
                let (avatar_did, handle, created) = self.world.ensure_avatar(
                    sender_did,
                    &sender_profile,
                    &sender_push_endpoint,
                    &sender_encryption_pubkey_multibase,
                    &room,
                ).await?;
                if created {
                    self.push_presence_snapshot(&room).await;
                }

                let route_room = match &envelope {
                    MessageEnvelope::ActorCommand { target, .. }
                        if target.eq_ignore_ascii_case("avatar") =>
                    {
                        self.world
                            .avatar_room_for_did(&avatar_did.id())
                            .await
                            .unwrap_or_else(|| room.clone())
                    }
                    _ => room.clone(),
                };

                let _ = self
                    .world
                    .touch_avatar_presence_for_did(&route_room, &avatar_did.id())
                    .await;
                self
                    .world
                    .upsert_avatar_location(&route_room, &avatar_did.id(), &sender_push_endpoint)
                    .await;
                let effective_sender_profile = self
                    .world
                    .avatar_language_order_for_did(&route_room, &avatar_did.id())
                    .await
                    .unwrap_or_else(|| "nb_NO:en_UK".to_string());
                let is_world_admin = matches!(
                    &envelope,
                    MessageEnvelope::ActorCommand { target, .. } if target.eq_ignore_ascii_case("world")
                );
                if is_world_admin && !self.world.is_world_target_did(&message.to).await {
                    return Err(anyhow!(
                        "@world commands must target this world DID; got to='{}'",
                        message.to
                    ));
                }

                let requested_world_broadcast = match &envelope {
                    MessageEnvelope::ActorCommand {
                        target,
                        command: ActorCommand::Raw { command },
                    } if target.eq_ignore_ascii_case("world") => {
                        let raw = command.trim();
                        if let Some(rest) = raw.strip_prefix("broadcast") {
                            let text = rest.trim();
                            if text.is_empty() {
                                None
                            } else {
                                Some(text.to_string())
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                // Route command envelopes using the active room handle bound to sender DID.
                let actor_name = self
                    .world
                    .avatar_handle_for_did(&route_room, &avatar_did.id())
                    .await
                    .unwrap_or_else(|| avatar_did.id());
                let (message, broadcasted, effective_room) = self
                    .world
                    .send_message(&route_room, &actor_name, &avatar_did, &effective_sender_profile, envelope)
                    .await?;
                if effective_room != route_room {
                    self.push_presence_room_state_to(&effective_room, &sender_push_endpoint).await;
                    self.push_presence_snapshot(&route_room).await;
                    self.push_presence_snapshot(&effective_room).await;
                }
                if let Some(text) = requested_world_broadcast {
                    if message.starts_with("@world broadcast sent") {
                        self.push_world_broadcast(&effective_room, &text).await;
                    }
                }
                let latest_event_sequence = self.world.latest_room_event_sequence(&effective_room).await?;
                if broadcasted {
                    let proto = self.clone();
                    let room_for_broadcast = effective_room.clone();
                    let seq_before = if latest_event_sequence > 0 { latest_event_sequence - 1 } else { 0 };
                    tokio::spawn(async move {
                        proto.push_room_events(&room_for_broadcast, seq_before).await;
                    });
                }
                Ok(WorldResponse {
                    ok: true,
                    room_description: self.world.room_description(&effective_room).await,
                    room_title: self.world.room_title(&effective_room).await,
                    room_did: self.world.room_did(&effective_room).await,
                    world_did: self.world.world_did.read().await.clone().unwrap_or_default(),
                    avatars: self.world.room_avatars(&effective_room).await,
                    room: effective_room.clone(),
                    message,
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence,
                    broadcasted,
                    events: Vec::new(),
                    handle,
                    room_object_dids: self.world.room_object_did_map(&effective_room).await,
                    transport_ack: None,
                })
            }
            WorldCommand::RoomEvents { room, since_sequence } => {
                let (avatar_did, _handle, created) = self.world.ensure_avatar(
                    sender_did,
                    &sender_profile,
                    &sender_push_endpoint,
                    &sender_encryption_pubkey_multibase,
                    &room,
                ).await?;
                if created {
                    self.push_presence_snapshot(&room).await;
                }
                let _ = self
                    .world
                    .touch_avatar_presence_for_did(&room, &avatar_did.id())
                    .await;
                self
                    .world
                    .upsert_avatar_location(&room, &avatar_did.id(), &sender_push_endpoint)
                    .await;
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
                    world_did: self.world.world_did.read().await.clone().unwrap_or_default(),
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
                    world_did: String::new(),
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

impl IpfsProtocol {
    async fn process_request(&self, request: WorldRequest) -> IpfsPublishDidResponse {
        match self.handle_request(request).await {
            Ok(response) => response,
            Err(err) => IpfsPublishDidResponse {
                ok: false,
                message: err.to_string(),
                did: None,
                key_name: None,
                cid: None,
            },
        }
    }

    async fn handle_request(&self, request: WorldRequest) -> Result<IpfsPublishDidResponse> {
        let validated = ma_core::ipfs_publish::validate_ipfs_publish_request(
            &request.message_cbor,
        )?;

        {
            let mut cache = self.did_cache.write().await;
            cache.insert(
                validated.document_did.ipns.clone(),
                CachedDidDocument {
                    document: validated.document.clone(),
                    dirty: false,
                },
            );
        }

        let (key_name, cid) = ma_core::ipfs_publish::publish_did_document_to_kubo(
            &self.kubo_url,
            &validated.request.did_document_json,
            &validated.request.ipns_private_key_base64,
            validated.request.desired_fragment.as_deref(),
        )
        .await?;

        Ok(IpfsPublishDidResponse {
            ok: true,
            message: "did document published via ma/ipfs/1".to_string(),
            did: Some(validated.document_did.id()),
            key_name,
            cid,
        })
    }
}

impl ProtocolHandler for IpfsProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;

        loop {
            let frame_len = match recv.read_u32().await {
                Ok(n) => n as usize,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(AcceptError::from_err(err)),
            };

            if frame_len > 1024 * 1024 {
                return Err(AcceptError::from_err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("ipfs frame too large: {}", frame_len),
                )));
            }

            let mut bytes = vec![0u8; frame_len];
            recv.read_exact(&mut bytes).await.map_err(AcceptError::from_err)?;

            let response = match serde_json::from_slice::<WorldRequest>(&bytes) {
                Ok(request) => self.process_request(request).await,
                Err(err) => IpfsPublishDidResponse {
                    ok: false,
                    message: format!("invalid ipfs request JSON: {}", err),
                    did: None,
                    key_name: None,
                    cid: None,
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

/// Resolve the world root CID from the ma.world IPLD link in the DID document.
async fn resolve_world_root_cid_from_did(kubo_url: &str, world_did: &str) -> Result<Option<String>> {
    let world = Did::try_from(world_did)
        .map_err(|e| anyhow!("invalid world DID '{}': {}", world_did, e))?;
    let document = kubo::fetch_did_document(kubo_url, &world).await?;
    let Some(ma) = document.ma.as_ref() else {
        return Ok(None);
    };
    // ma.world is an IPLD link { "/": "<cid>" }
    let Some(world_val) = ma.world.as_ref() else {
        return Ok(None);
    };
    let Some(obj) = world_val.as_object() else {
        return Ok(None);
    };
    let Some(cid_str) = obj.get("/").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let trimmed = cid_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

async fn publish_world_did_runtime_ma(
    kubo_url: &str,
    world_slug: &str,
    world_master_key: [u8; 32],
    state_cid: &str,
    root_cid: &str,
) -> Result<()> {
    const RUNTIME_IPNS_ATTEMPTS: u32 = 3;
    const RUNTIME_IPNS_TIMEOUT_SECS: u64 = 12;
    const RUNTIME_IPNS_BACKOFF_MS: u64 = 500;

    let world_key_name = normalize_world_key_name(world_slug);
    let did_identifier = ensure_kubo_key_id(kubo_url, &world_key_name).await?;
    let world_did = Did::new(&did_identifier, world_slug)
        .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;

    let signing_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build signing DID: {}", e))?;
    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        derive_world_signing_private_key(&world_master_key),
    )
    .map_err(|e| anyhow!("failed to restore world signing key: {}", e))?;

    let mut document = kubo::fetch_did_document(kubo_url, &world_did).await?;
    document.set_ma_type("world")?;

    // Set ma.world as an IPLD link to the world root CID for DAG traversal.
    document.set_ma_world(serde_json::json!({ "/": root_cid }));
    document.set_ma_state_cid(state_cid);

    let assertion_id = document.assertion_method.first()
        .ok_or_else(|| anyhow!("world DID has no assertionMethod"))?
        .clone();
    let assertion_vm = document
        .get_verification_method_by_id(&assertion_id)
        .map_err(|e| anyhow!("world DID missing assertion method '{}': {}", assertion_id, e))?
        .clone();
    document.sign(&signing_key, &assertion_vm)?;

    let document_cid = dag_put_dag_cbor(kubo_url, &document).await?;

    let ipns_ttl_secs = std::env::var("MA_WORLD_IPNS_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let ipns_options = IpnsPublishOptions {
        timeout: Duration::from_secs(RUNTIME_IPNS_TIMEOUT_SECS),
        ttl: ipns_ttl_secs.map(|s| format!("{}s", s)),
        ..IpnsPublishOptions::default()
    };
    ipns_publish_with_retry(
        kubo_url,
        &world_key_name,
        &document_cid,
        &ipns_options,
        RUNTIME_IPNS_ATTEMPTS,
        Duration::from_millis(RUNTIME_IPNS_BACKOFF_MS),
    )
    .await?;

    Ok(())
}

async fn ensure_world_did_document(
    kubo_url: &str,
    world_slug: &str,
    endpoint_id: &str,
    world_master_key: [u8; 32],
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

    let world_did = Did::new(&did_identifier, world_slug)
        .map_err(|e| anyhow!("failed to build world DID from IPNS key '{}' slug '{}': {}", did_identifier, world_slug, e))?;

    let signing_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build signing DID: {}", e))?;
    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        derive_world_signing_private_key(&world_master_key),
    )
        .map_err(|e| anyhow!("failed to restore world signing key: {}", e))?;

    let key_agreement_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build key-agreement DID: {}", e))?;
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
        signing_key.did.fragment.as_deref().unwrap_or_default(),
        signing_key.public_key_multibase.clone(),
    )
    .map_err(|e| anyhow!("failed building world assertion method: {}", e))?;

    let key_agreement_vm = VerificationMethod::new(
        world_did.base_id(),
        world_did.base_id(),
        key_agreement_key.key_type.clone(),
        key_agreement_key.did.fragment.as_deref().unwrap_or_default(),
        key_agreement_key.public_key_multibase.clone(),
    )
    .map_err(|e| anyhow!("failed building world keyAgreement method: {}", e))?;

    let assertion_vm_id = assertion_vm.id.clone();
    let key_agreement_vm_id = key_agreement_vm.id.clone();
    document.add_verification_method(assertion_vm.clone())?;
    document.add_verification_method(key_agreement_vm)?;
    document.assertion_method = vec![assertion_vm_id];
    document.key_agreement = vec![key_agreement_vm_id];
    document.set_ma_type("world")?;
    let transport_paths = vec![
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(INBOX_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(AVATAR_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(IPFS_ALPN)),
    ];
    document.set_ma_transports(serde_json::Value::Array(
        transport_paths
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    ));
    document.set_ma_ping_interval_secs(WORLD_PING_INTERVAL_SECS);
    document.sign(&signing_key, &assertion_vm)?;

    let document_cid = dag_put_dag_cbor(kubo_url, &document).await?;

    info!(
        "World DID document {} stored as CID {} — IPNS publish continues in background",
        world_did.base_id(),
        document_cid
    );

    // Spawn IPNS publish in background so startup is not blocked by slow Kubo publishes.
    let bg_kubo_url = kubo_url.to_string();
    let bg_key_name = key_name.clone();
    let bg_document_cid = document_cid.clone();
    let bg_world_did_id = world_did.base_id();
    tokio::spawn(async move {
        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_secs(45),
            ..IpnsPublishOptions::default()
        };
        match ipns_publish_with_retry(
            &bg_kubo_url,
            &bg_key_name,
            &bg_document_cid,
            &ipns_options,
            8,
            Duration::from_millis(1500),
        )
        .await
        {
            Ok(published) => info!(
                "Background IPNS publish complete for {} CID {} (IPNS {})",
                bg_world_did_id, bg_document_cid, published
            ),
            Err(err) => warn!(
                "Background IPNS publish failed for {} CID {}: {}",
                bg_world_did_id, bg_document_cid, err
            ),
        }
    });

    Ok(world_did.base_id())
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
    let mut owner_override: Option<String> = None;

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
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug = args[idx].clone();
                        }
                        other => {
                            if explicit_path.is_some() {
                                return Err(anyhow!(
                                    "usage: ma-world --gen-iroh-secret [path] [--slug <slug>]"
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
            "--gen-headless-config" => {
                let mut world_slug: Option<String> = None;
                let mut passphrase: Option<String> = None;
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug = Some(args[idx].clone());
                        }
                        "--passphrase" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --passphrase"));
                            }
                            passphrase = Some(args[idx].clone());
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for --gen-headless-config (supported: --slug, --passphrase)",
                                other
                            ));
                        }
                    }
                    idx += 1;
                }

                let world_slug = world_slug
                    .ok_or_else(|| anyhow!("--slug is required for --gen-headless-config"))?;
                let normalized_slug = normalize_world_key_name(&world_slug);
                let runtime_cfg_path = runtime_config_path(&normalized_slug);
                let cfg_dir = runtime_cfg_path
                    .parent()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow!("invalid runtime config path {}", runtime_cfg_path.display()))?;
                ensure_private_dir(&cfg_dir)?;

                let iroh_path = cfg_dir.join(format!("{}_iroh.bin", normalized_slug));
                let bundle_path = cfg_dir.join(format!("{}_bundle.json", normalized_slug));
                let config_path = cfg_dir.join(format!("{}.yaml", normalized_slug));

                if config_path.exists() {
                    return Err(anyhow!("config already exists at {}", config_path.display()));
                }

                let passphrase = passphrase.unwrap_or_else(|| nanoid!(32));

                generate_iroh_secret_file(&iroh_path)?;

                let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
                    .ok_or_else(|| anyhow!("failed loading generated iroh secret {}", iroh_path.display()))?;
                let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);

                let world = World::new(
                    EntryAcl {
                        allow_all: true,
                        allow_owner: true,
                        allowed_dids: HashSet::new(),
                        source: "*".to_string(),
                    },
                    DEFAULT_KUBO_API_URL.to_string(),
                    normalized_slug.clone(),
                );
                world.set_world_master_key(world_master_key).await;
                let bundle_json = world.create_unlock_bundle(&passphrase).await?;
                write_secure_file(&bundle_path, bundle_json.as_bytes(), SecureFileKind::SensitiveData)?;

                let cfg = bootstrap::RuntimeFileConfig {
                    iroh_secret: Some(iroh_path.display().to_string()),
                    status_api_enabled: Some(true),
                    admin_api_enabled: Some(false),
                    unlock_passphrase: Some(passphrase.clone()),
                    unlock_bundle_file: Some(bundle_path.display().to_string()),
                    ..Default::default()
                };
                let yaml = serde_yaml::to_string(&cfg)?;
                write_secure_file(&config_path, yaml.as_bytes(), SecureFileKind::RuntimeConfig)?;

                println!("generated headless config artifacts for slug '{}':", normalized_slug);
                println!("  iroh_secret: {}", iroh_path.display());
                println!("  unlock_bundle_file: {}", bundle_path.display());
                println!("  config: {}", config_path.display());
                println!("  passphrase: {}", passphrase);
                return Ok(());
            }
            "create-unlock-bundle" => {
                let mut world_slug = DEFAULT_WORLD_SLUG.to_string();
                let mut passphrase: Option<String> = None;
                let mut out_path: Option<PathBuf> = None;
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug = args[idx].clone();
                        }
                        "--passphrase" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --passphrase"));
                            }
                            passphrase = Some(args[idx].clone());
                        }
                        "--out" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --out"));
                            }
                            out_path = Some(PathBuf::from(&args[idx]));
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for create-unlock-bundle (supported: --slug, --passphrase, --out)",
                                other
                            ));
                        }
                    }
                    idx += 1;
                }

                let passphrase = passphrase
                    .ok_or_else(|| anyhow!("--passphrase is required for create-unlock-bundle"))?;
                let normalized_slug = normalize_world_key_name(&world_slug);
                let runtime_cfg_path = runtime_config_path(&normalized_slug);
                let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
                let iroh_path = runtime_cfg
                    .iroh_secret
                    .map(PathBuf::from)
                    .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));
                let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
                    .ok_or_else(|| anyhow!(
                        "missing iroh secret at {}. Create it with: ma-world --gen-iroh-secret --slug {}",
                        iroh_path.display(),
                        normalized_slug
                    ))?;
                let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);

                let world = World::new(
                    EntryAcl {
                        allow_all: true,
                        allow_owner: true,
                        allowed_dids: HashSet::new(),
                        source: "*".to_string(),
                    },
                    DEFAULT_KUBO_API_URL.to_string(),
                    normalized_slug.clone(),
                );
                world.set_world_master_key(world_master_key).await;
                let bundle_json = world.create_unlock_bundle(&passphrase).await?;

                let out_path = out_path.unwrap_or_else(|| {
                    runtime_config_path(&normalized_slug)
                        .with_file_name(format!("{}_bundle.json", normalized_slug))
                });
                write_secure_file(&out_path, bundle_json.as_bytes(), SecureFileKind::SensitiveData)?;
                println!("created unlock bundle: {}", out_path.display());
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
                        "--owner" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --owner"));
                            }
                            owner_override = Some(args[idx].clone());
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
                            log_file = Some(expand_tilde_path(&args[idx]));
                        }
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug_override = Some(args[idx].clone());
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for run (supported: --slug, --listen, --kubo-url, --owner, --log-level, --log-file)",
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
                "--owner" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --owner"));
                    }
                    owner_override = Some(args[idx].clone());
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
                    log_file = Some(expand_tilde_path(&args[idx]));
                }
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug_override = Some(args[idx].clone());
                }
                other => {
                    return Err(anyhow!(
                        "unknown top-level argument '{}'. Use 'publish-world' for publish flags like --skip-ipns, or use run/--slug/--listen/--kubo-url/--owner/--log-level/--log-file for server mode.",
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
                .ok_or_else(|| anyhow!("--slug is required for server mode"))?,
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

        if owner_override.is_none() {
            if let Some(cfg_owner) = runtime_file_config.owner.clone() {
                owner_override = Some(cfg_owner);
            } else if let Ok(env_owner) = std::env::var("MA_WORLD_OWNER") {
                owner_override = Some(env_owner);
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
                log_file = Some(expand_tilde_path(&cfg_file));
            } else if let Ok(env_file) = std::env::var("MA_LOG_FILE") {
                log_file = Some(expand_tilde_path(&env_file));
            } else {
                log_file = Some(
                    xdg_data_home()
                        .join("ma")
                        .join("worlds")
                        .join(&normalized_slug)
                        .join("ma-world.log"),
                );
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
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
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
                        "unknown argument '{}' for check-kubo-ipns (supported: --slug, --world-dir, --key, --ipns-timeout-ms, --ipns-retries, --ipns-backoff-ms)",
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
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
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
                        "unknown argument '{}' for ensure-kubo-keys (supported: --slug, --world-dir)",
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
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
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
                        "unknown argument '{}' for check-kubo-keys (supported: --slug, --world-dir)",
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
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
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
                        "unknown argument '{}' for publish-world (supported: --slug, --world-dir, --skip-ipns, --allow-partial-ipns, --ipns-timeout-ms, --ipns-retries, --ipns-backoff-ms)",
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
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
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
                        "unknown argument '{}' for validate-world (supported: --slug, --world-dir)",
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

    if args.len() >= 2 && args[1] == "verify-root" {
        let mut world_slug = default_world_slug.clone();
        let mut root_cid: Option<String> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--root-cid" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --root-cid"));
                    }
                    root_cid = Some(args[idx].clone());
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for verify-root (supported: --slug, --root-cid)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let root_cid = root_cid.ok_or_else(|| anyhow!("--root-cid is required for verify-root"))?;
        let normalized_slug = normalize_world_key_name(&world_slug);
        let runtime_cfg_path = runtime_config_path(&normalized_slug);
        let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
        let kubo_url = runtime_cfg
            .kubo_api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_KUBO_API_URL.to_string());
        let iroh_path = runtime_cfg
            .iroh_secret
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));
        let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
            .ok_or_else(|| anyhow!(
                "missing iroh secret at {}. Create it with: ma-world --gen-iroh-secret --slug {}",
                iroh_path.display(),
                normalized_slug
            ))?;
        let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);
        let world_key_name = normalize_world_key_name(&normalized_slug);
        let did_identifier = ensure_kubo_key_id(&kubo_url, &world_key_name).await?;
        let world_did = Did::new(&did_identifier, &normalized_slug)
            .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;

        let world = World::new(
            EntryAcl {
                allow_all: true,
                allow_owner: true,
                allowed_dids: HashSet::new(),
                source: "*".to_string(),
            },
            kubo_url.clone(),
            normalized_slug.clone(),
        );
        world.set_world_master_key(world_master_key).await;
        world.set_world_did(&world_did.id()).await?;

        let rooms_loaded = world.load_from_world_cid(&root_cid).await?;
        let mut verified_state_cid = String::new();
        if let Some(state_cid) = world.state_cid().await {
            let _ = world.load_encrypted_state(&state_cid).await?;
            verified_state_cid = state_cid;
        }

        println!("verify-root OK");
        println!("  slug: {}", normalized_slug);
        println!("  root_cid: {}", root_cid);
        println!("  rooms_loaded: {}", rooms_loaded);
        if verified_state_cid.is_empty() {
            println!("  state_cid: (none)");
        } else {
            println!("  state_cid: {}", verified_state_cid);
        }
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "restore-root" {
        let mut world_slug = default_world_slug.clone();
        let mut root_cid: Option<String> = None;
        let mut dry_run = false;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--root-cid" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --root-cid"));
                    }
                    root_cid = Some(args[idx].clone());
                }
                "--dry-run" => {
                    dry_run = true;
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for restore-root (supported: --slug, --root-cid, --dry-run)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let root_cid = root_cid.ok_or_else(|| anyhow!("--root-cid is required for restore-root"))?;
        let normalized_slug = normalize_world_key_name(&world_slug);
        let runtime_cfg_path = runtime_config_path(&normalized_slug);
        let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
        let kubo_url = runtime_cfg
            .kubo_api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_KUBO_API_URL.to_string());
        let iroh_path = runtime_cfg
            .iroh_secret
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));
        let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
            .ok_or_else(|| anyhow!(
                "missing iroh secret at {}. Create it with: ma-world --gen-iroh-secret --slug {}",
                iroh_path.display(),
                normalized_slug
            ))?;
        let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);
        let world_key_name = normalize_world_key_name(&normalized_slug);
        let did_identifier = ensure_kubo_key_id(&kubo_url, &world_key_name).await?;
        let world_did = Did::new(&did_identifier, &normalized_slug)
            .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;

        let world = World::new(
            EntryAcl {
                allow_all: true,
                allow_owner: true,
                allowed_dids: HashSet::new(),
                source: "*".to_string(),
            },
            kubo_url.clone(),
            normalized_slug.clone(),
        );
        world.set_world_master_key(world_master_key).await;
        world.set_world_did(&world_did.id()).await?;

        let rooms_loaded = world.load_from_world_cid(&root_cid).await?;
        if let Some(state_cid) = world.state_cid().await {
            let _ = world.load_encrypted_state(&state_cid).await?;
        }

        if dry_run {
            println!("restore-root DRY-RUN OK");
            println!("  slug: {}", normalized_slug);
            println!("  input_root_cid: {}", root_cid);
            println!("  rooms_loaded: {}", rooms_loaded);
            return Ok(());
        }

        let (new_state_cid, new_root_cid) = world.save_and_publish().await?;

        println!("restore-root OK");
        println!("  slug: {}", normalized_slug);
        println!("  input_root_cid: {}", root_cid);
        println!("  rooms_loaded: {}", rooms_loaded);
        println!("  output_state_cid: {}", new_state_cid);
        println!("  output_root_cid: {}", new_root_cid);
        return Ok(());
    }

    if args.len() >= 2 && !run_arg_mode {
        return Err(anyhow!(
            "unknown command '{}'. Use --help to list available commands.",
            args[1]
        ));
    }

    // Initialize logging with configurable level and optional file output.
    // Default filters keep normal runs readable while allowing richer transport traces in debug mode.
    let normalized_level = log_level.trim().to_lowercase();
    let iroh_level = if normalized_level == "debug" || normalized_level == "trace" {
        normalized_level.as_str()
    } else {
        "info"
    };
    let directives = [
        format!("ma_world={}", normalized_level),
        format!("ma_core={}", normalized_level),
        format!("iroh={}", iroh_level),
        format!("iroh_net={}", iroh_level),
        format!("iroh_relay={}", iroh_level),
    ];
    let mut env_filter = tracing_subscriber::EnvFilter::from_default_env();
    for directive in directives {
        env_filter = env_filter.add_directive(directive.parse()?);
    }

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
        .ok_or_else(|| anyhow!("--slug is required for server mode"))?;
    let world_slug = normalize_world_key_name(&runtime_slug);
    let runtime_cfg_path = runtime_config_path(&world_slug);
    let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
    let authored_world_dir = default_world_dir(&world_slug);
    let authored_world = load_world_authoring(&authored_world_dir).ok();
    let authored_global_acl_cid = authored_world
        .as_ref()
        .and_then(|loaded| loaded.world_root.refs.global_acl_cid.clone())
        .filter(|cid| !cid.trim().is_empty());

    let kubo_url = kubo_url_override
        .or_else(|| runtime_cfg.kubo_api_url.clone())
        .or_else(|| std::env::var("MA_KUBO_API_URL").ok())
        .unwrap_or_else(|| default_kubo_url.clone());
    let status_api_enabled = runtime_cfg.status_api_enabled.unwrap_or(true);
    let admin_api_enabled = runtime_cfg.admin_api_enabled.unwrap_or(false);
    let admin_api_token = if admin_api_enabled {
        Some(
            std::env::var("MA_WORLD_ADMIN_API_TOKEN")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    runtime_cfg
                        .admin_api_token
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                })
                .ok_or_else(|| {
                    anyhow!(
                        "missing admin API token: set admin_api_token in {} or MA_WORLD_ADMIN_API_TOKEN",
                        runtime_cfg_path.display()
                    )
                })?,
        )
    } else {
        None
    };
    let entry_acl = load_entry_acl()?;
    let world = Arc::new(World::new(
        entry_acl,
        kubo_url.clone(),
        world_slug.clone(),
    ));
    info!("Runtime world slug: {}", world_slug);
    info!("Runtime config path: {}", runtime_cfg_path.display());
    info!("Status API enabled: {}", status_api_enabled);
    info!("Admin API enabled: {}", admin_api_enabled);

    if let Some(owner) = owner_override.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        world.set_owner_did(owner).await?;
    }

    world.create_room(DEFAULT_ROOM.to_string()).await?;

    // Best-effort local auto-unlock for single-user dev/test flow.
    // If authoring + key material are present, start in unlocked mode without status-page interaction.
    if let Ok(loaded_for_unlock) = load_world_authoring(&authored_world_dir) {
        let master_key_rel = PathBuf::from(loaded_for_unlock.config.crypto.world_master_key_file.clone());
        let master_key_path = if master_key_rel.is_absolute() {
            master_key_rel
        } else {
            loaded_for_unlock.world_dir.join(master_key_rel)
        };

        world
            .set_unlock_context(loaded_for_unlock.world_dir.clone(), master_key_path.clone())
            .await;

        match fs::read(&master_key_path) {
            Ok(bytes) => {
                let master_key: Result<[u8; 32], _> = bytes.as_slice().try_into();
                match master_key {
                    Ok(key) => {
                        world.set_world_master_key(key).await;
                        match unlock_actor_secret_bundles(&loaded_for_unlock) {
                            Ok(bundles) => {
                                let count = bundles.len();
                                if let Err(err) = world.install_actor_secrets(&bundles).await {
                                    warn!("auto-unlock: failed installing actor secrets: {}", err);
                                } else {
                                    *world.unlocked.write().await = true;
                                    info!("auto-unlock: enabled with {} actor bundles", count);
                                }
                            }
                            Err(err) => {
                                warn!("auto-unlock: failed unlocking actor secret bundles: {}", err);
                            }
                        }
                    }
                    Err(_) => {
                        warn!(
                            "auto-unlock: world master key must be 32 bytes in {}",
                            master_key_path.display()
                        );
                    }
                }
            }
            Err(err) => {
                warn!(
                    "auto-unlock: could not read world master key {}: {}",
                    master_key_path.display(),
                    err
                );
            }
        }
    }

    // Passphrase-based auto-unlock from runtime config.
    if !world.is_unlocked().await {
        if let Some(passphrase) = runtime_cfg.unlock_passphrase.as_deref().filter(|s| !s.trim().is_empty()) {
            let bundle_file = runtime_cfg
                .unlock_bundle_file
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    runtime_config_path(&world_slug)
                        .with_file_name(format!("{}_bundle.json", world_slug))
                });
            match fs::read_to_string(&bundle_file) {
                Ok(bundle_json) => {
                    match world.unlock_runtime(passphrase, &bundle_json).await {
                        Ok(count) => {
                            info!(
                                "auto-unlock (passphrase): enabled with {} actor bundles from {}",
                                count,
                                bundle_file.display()
                            );
                        }
                        Err(err) => {
                            warn!(
                                "auto-unlock (passphrase): unlock failed for bundle {}: {}",
                                bundle_file.display(),
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "auto-unlock (passphrase): could not read bundle {}: {}",
                        bundle_file.display(),
                        err
                    );
                }
            }
        }
    }

    // Bind status listener before iroh endpoint setup so listen failures abort early.
    let listener = if status_api_enabled {
        Some(bind_status_listener(&listen_addr).await?)
    } else {
        None
    };
    let status_url = listener
        .as_ref()
        .map(|socket| socket.local_addr())
        .transpose()?
        .map(|addr| format!("http://{}/status.json", addr))
        .unwrap_or_else(|| "disabled".to_string());

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
    wait_for_kubo_api(&kubo_url, 8).await?;

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

    let run_result: Result<()> = async {
        let world_master_key = derive_world_master_key(endpoint.secret_key(), &world_slug);
        world.set_world_master_key(world_master_key).await;
        info!("World master key source: derived from iroh identity and world slug");

        let endpoint_id = endpoint.id().to_string();
        let world_did = ensure_world_did_document(
            &kubo_url,
            &world_slug,
            &endpoint_id,
            world_master_key,
        )
        .await?;
        world.set_world_did(&world_did).await?;
        info!("Runtime world DID: {}", world_did);

        let restore_root = match resolve_world_root_cid_from_did(&kubo_url, &world_did).await {
            Ok(r) => r,
            Err(err) => {
                warn!("Failed resolving world root CID from DID {}: {} — starting fresh", world_did, err);
                None
            }
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
                "Bootstrapped world state with lobby snapshot: state_cid={} root_cid={} — IPNS deferred to background publisher",
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

        // Background IPNS publisher: periodically publishes to IPNS if dirty.
        {
            let ipns_interval = std::env::var("MA_WORLD_IPNS_PUBLISH_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(IPNS_PUBLISH_INTERVAL_SECS_DEFAULT);
            let world_for_ipns = world.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(ipns_interval));
                ticker.tick().await; // skip immediate first tick
                loop {
                    ticker.tick().await;
                    if !world_for_ipns.is_ipns_dirty().await {
                        continue;
                    }
                    info!("IPNS publish timer: dirty state detected, publishing...");
                    match world_for_ipns.publish_to_ipns().await {
                        Ok(()) => info!("IPNS publish timer: published successfully"),
                        Err(err) => warn!("IPNS publish timer: failed: {}", err),
                    }
                }
            });
        }

        let did_cache = Arc::new(RwLock::new(HashMap::new()));
        let ipfs_protocol = IpfsProtocol {
            kubo_url: kubo_url.clone(),
            did_cache: did_cache.clone(),
        };
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
                LaneCapability::for_lane(WorldLane::Inbox),
                LaneCapability::for_lane(WorldLane::Avatar),
            ],
        };

        if let Some(listener) = listener {
            let status_world = world.clone();
            let status_info = world_info.clone();
            let status_www_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("www");
            let admin_api_token = admin_api_token.clone();
            tokio::spawn(async move {
                if let Err(err) = status::serve(
                    listener,
                    status_world,
                    status_info,
                    status_www_root,
                    admin_api_enabled,
                    admin_api_token,
                )
                .await
                {
                    error!("status server failed: {}", err);
                }
            });
        }

        info!("Created default room: {}", DEFAULT_ROOM);
        info!("World endpoint id: {}", world_info.endpoint_id);
        info!("World status page: {}", world_info.status_url);
        info!("Inbox protocol ALPN: {}", String::from_utf8_lossy(INBOX_ALPN));
        info!("IPFS protocol ALPN (citizenship): {}", String::from_utf8_lossy(IPFS_ALPN));
        info!("Presence protocol ALPN (outbound push to agents): {}", String::from_utf8_lossy(PRESENCE_ALPN));
        info!("World entry ACL: {}", world_info.entry_acl);
        info!("Optional DID field ma:presenceHint = {}", world_info.location_hint);
        info!("Iroh online readiness: {}", online_status);

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

        // Wait for unlock before accepting protocol connections.
        // While locked, a gate router responds with "world is locked" on all ALPNs.
        if !world.is_unlocked().await {
            let gate = LockedGateProtocol;
            let gate_router = Router::builder(endpoint.clone())
                .accept(INBOX_ALPN, gate.clone())
                .accept(AVATAR_ALPN, gate.clone())
                .accept(IPFS_ALPN, gate)
                .spawn();
            world
                .record_event("world runtime locked; gate router active — waiting for unlock".to_string())
                .await;
            info!("World locked — gate router active, waiting for unlock via status page at {}", world_info.status_url);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if world.is_unlocked().await {
                    break;
                }
            }
            gate_router.shutdown().await?;
            world
                .record_event("world unlocked — starting protocol lanes".to_string())
                .await;
            info!("World unlocked — starting protocol lanes");
        }

        let inbox_protocol = WorldProtocol {
            world: world.clone(),
            endpoint: endpoint.clone(),
            endpoint_id: endpoint_id.clone(),
            did_cache: did_cache.clone(),
            push_stream_cache: Arc::new(Mutex::new(HashMap::new())),
            push_timeout_cooldown: Arc::new(Mutex::new(HashMap::new())),
            lane: WorldLane::Inbox,
        };
        let avatar_protocol = WorldProtocol {
            world: world.clone(),
            endpoint: endpoint.clone(),
            endpoint_id: endpoint_id.clone(),
            did_cache: did_cache.clone(),
            push_stream_cache: inbox_protocol.push_stream_cache.clone(),
            push_timeout_cooldown: inbox_protocol.push_timeout_cooldown.clone(),
            lane: WorldLane::Avatar,
        };
        let router = Router::builder(endpoint.clone())
            .accept(INBOX_ALPN, inbox_protocol.clone())
            .accept(AVATAR_ALPN, avatar_protocol)
            .accept(IPFS_ALPN, ipfs_protocol)
            .spawn();

        let presence_probe_secs = env::var("MA_PRESENCE_PROBE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(PRESENCE_PROBE_INTERVAL_SECS_DEFAULT);
        let presence_stale_secs = env::var("MA_PRESENCE_STALE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 2)
            .unwrap_or(PRESENCE_STALE_AFTER_SECS_DEFAULT)
            .max(presence_probe_secs + 1);

        {
            let mut locations = world.avatar_locations.write().await;
            locations.set_default_max_cache(Duration::from_secs(presence_stale_secs));
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let inbox_presence = inbox_protocol.clone();
        let mut probe_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(presence_probe_secs));
            let stale_after = Duration::from_secs(presence_stale_secs);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = probe_shutdown.changed() => { break; }
                }
                if *probe_shutdown.borrow() {
                    break;
                }
                let changed_rooms = inbox_presence.world.prune_stale_avatars(stale_after).await;
                for room_name in changed_rooms {
                    inbox_presence.push_presence_snapshot(&room_name).await;
                }
            }
        });

        let refresh_protocol = inbox_protocol.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let room_names = {
                let rooms = refresh_protocol.world.rooms.read().await;
                rooms.keys().cloned().collect::<Vec<_>>()
            };
            for room_name in room_names {
                refresh_protocol.push_presence_refresh_request(&room_name).await;
            }
        });

        info!(
            "Presence pruning enabled: probe={}s stale_after={}s",
            presence_probe_secs,
            presence_stale_secs
        );

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
            .record_event(format!("inbox protocol ready on ALPN {}", String::from_utf8_lossy(INBOX_ALPN)))
            .await;

        info!("World initialized. Waiting for connections...");
        let signal_name = wait_for_shutdown_signal().await?;
        info!("Received {} shutting down", signal_name);
        info!("Shutting down ma-world — stopping background tasks...");

        // Cancel the presence probe loop immediately so it stops trying to
        // reach stale/dead actor endpoints before we close the iroh router.
        let _ = shutdown_tx.send(true);

        info!("Shutting down ma-world — saving state...");
        match world.save_and_publish().await {
            Ok((state_cid, root_cid)) => {
                info!("State saved and published: state_cid={} root_cid={}", state_cid, root_cid);
            }
            Err(e) => {
                warn!("Failed to save state on shutdown: {}", e);
            }
        }

        // Give the iroh router a bounded window to drain open connections.
        // If it takes longer than 5 s we exit anyway — connections will time out
        // on the actor side.
        let shutdown_timeout = Duration::from_secs(5);
        match tokio::time::timeout(shutdown_timeout, router.shutdown()).await {
            Ok(Ok(())) => info!("Router shut down cleanly."),
            Ok(Err(e)) => warn!("Router shutdown error: {}", e),
            Err(_) => warn!("Router shutdown timed out after {}s; forcing exit.", shutdown_timeout.as_secs()),
        }

        Ok(())
    }
    .await;

    endpoint.close().await;
    run_result
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<&'static str> {
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok("SIGINT"),
        _ = sigterm.recv() => Ok("SIGTERM"),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<&'static str> {
    tokio::signal::ctrl_c().await?;
    Ok("SIGINT")
}

async fn bind_status_listener(listen_addr: &str) -> Result<TcpListener> {
    let socket: SocketAddr = listen_addr
        .parse()
        .map_err(|e| anyhow!("invalid --listen '{}': {}", listen_addr, e))?;
    TcpListener::bind(socket)
        .await
        .map_err(|e| anyhow!("failed to bind status listener on {}: {}", socket, e))
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
        allowed_dids.insert(did.id());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_world() -> World {
        World::new(
            EntryAcl {
                allow_all: true,
                allow_owner: false,
                allowed_dids: HashSet::new(),
                source: "test".to_string(),
            },
            "http://127.0.0.1:5001".to_string(),
            "test-world".to_string(),
        )
    }

    #[test]
    fn extracts_did_description_from_supported_paths() {
        let ma_json = r#"{"ma":{"description":"hello from ma"}}"#;
        assert_eq!(
            extract_did_description_from_json(ma_json),
            Some("hello from ma".to_string())
        );

        let top_json = r#"{"description":"hello top"}"#;
        assert_eq!(
            extract_did_description_from_json(top_json),
            Some("hello top".to_string())
        );
    }

    #[tokio::test]
    async fn join_room_keeps_avatar_in_single_room() {
        let world = test_world();
        world.create_room("lobby".to_string()).await.unwrap();
        world.create_room("hall".to_string()).await.unwrap();

        let did = Did::try_from("did:ma:k51test#pixie").unwrap();
        let req = AvatarRequest {
            did: did.clone(),
            owner: "did:ma:k51test".to_string(),
            agent_endpoint: "ep-1".to_string(),
            language_order: "nb_NO:en_UK".to_string(),
            signing_secret: [0u8; 32],
            encryption_pubkey_multibase: None,
        };
        world.join_room("lobby", req, Some("pixie".to_string())).await.unwrap();

        let req2 = AvatarRequest {
            did,
            owner: "did:ma:k51test".to_string(),
            agent_endpoint: "ep-2".to_string(),
            language_order: "nb_NO:en_UK".to_string(),
            signing_secret: [0u8; 32],
            encryption_pubkey_multibase: None,
        };
        world.join_room("hall", req2, Some("pixie".to_string())).await.unwrap();

        assert_eq!(
            world.avatar_room_for_did("did:ma:k51test#pixie").await,
            Some("hall".to_string())
        );

        let rooms = world.rooms.read().await;
        let lobby = rooms.get("lobby").unwrap();
        let hall = rooms.get("hall").unwrap();
        assert!(lobby.avatars.is_empty());
        assert_eq!(hall.avatars.len(), 1);
    }

    #[tokio::test]
    async fn prune_stale_avatars_clears_membership_index() {
        let world = test_world();
        world.create_room("lobby".to_string()).await.unwrap();

        let did = Did::try_from("did:ma:k51stale#agent").unwrap();
        let req = AvatarRequest {
            did,
            owner: "did:ma:k51stale".to_string(),
            agent_endpoint: "ep-stale".to_string(),
            language_order: "nb_NO:en_UK".to_string(),
            signing_secret: [0u8; 32],
            encryption_pubkey_multibase: None,
        };
        world.join_room("lobby", req, Some("agent".to_string())).await.unwrap();

        {
            let mut rooms = world.rooms.write().await;
            let lobby = rooms.get_mut("lobby").unwrap();
            for avatar in lobby.avatars.values_mut() {
                avatar.last_seen_at = SystemTime::now()
                    .checked_sub(Duration::from_secs(90))
                    .unwrap();
            }
        }

        let changed = world.prune_stale_avatars(Duration::from_secs(25)).await;
        assert!(changed.iter().any(|room| room == "lobby"));
        assert_eq!(world.avatar_room_for_did("did:ma:k51stale#agent").await, None);
    }

    #[tokio::test]
    async fn world_room_owner_admin_override_sets_owner() {
        let world = test_world();
        world.create_room("lobby".to_string()).await.unwrap();
        world
            .set_owner_did("did:ma:k51admin#owner")
            .await
            .unwrap();

        let caller = Did::try_from("did:ma:k51admin#owner").unwrap();
        let response = world
            .handle_world_command(
                "lobby",
                "owner",
                &caller,
                "nb_NO:en_UK",
                "room lobby owner did:ma:k51recovered#hero",
            )
            .await;

        assert!(response.contains("owner set to did:ma:k51recovered#hero"));

        let rooms = world.rooms.read().await;
        let lobby = rooms.get("lobby").unwrap();
        assert_eq!(lobby.acl.owner.as_deref(), Some("did:ma:k51recovered#hero"));
        assert!(lobby.acl.allow.contains("did:ma:k51recovered#hero"));
        assert!(!lobby.acl.deny.contains("did:ma:k51recovered#hero"));
    }

    #[tokio::test]
    async fn world_room_owner_admin_override_rejects_fragmentless_did() {
        let world = test_world();
        world.create_room("lobby".to_string()).await.unwrap();
        world
            .set_owner_did("did:ma:k51admin#owner")
            .await
            .unwrap();

        let caller = Did::try_from("did:ma:k51admin#owner").unwrap();
        let response = world
            .handle_world_command(
                "lobby",
                "owner",
                &caller,
                "nb_NO:en_UK",
                "room lobby owner did:ma:k51recovered",
            )
            .await;

        assert!(response.contains("missing #fragment"));

        let rooms = world.rooms.read().await;
        let lobby = rooms.get("lobby").unwrap();
        assert_ne!(lobby.acl.owner.as_deref(), Some("did:ma:k51recovered"));
    }
}





