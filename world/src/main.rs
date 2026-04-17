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
    save_runtime_file_config,
};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use chrono::Utc;
use did_ma::{DID_PREFIX, Did, Document, EncryptionKey, Message, SigningKey, VerificationMethod};
use ma_core::{
    ActorCommand, AVATAR_PROTOCOL, PRESENCE_PROTOCOL,
    CONTENT_TYPE_BROADCAST, CONTENT_TYPE_EVENT, CONTENT_TYPE_PRESENCE, CONTENT_TYPE_WORLD,
    CompiledCapabilityAcl, ExitData, ServiceCapability, MessageEnvelope,
    ObjectDefinition, ObjectInboxMessage, MAILBOX_COMMANDS_INLINE,
    IpfsPublishDidResponse,
    ObjectMessageIntent, ObjectMessageKind, ObjectMessageRetention, ObjectMessageTarget,
    ObjectRuntimeState, IPFS_PROTOCOL, PresenceAvatar, RoomActorAction, RoomActorContext,
    RoomEvent, TransportAck, TransportAckCode, INBOX_PROTOCOL, WorldCommand, WorldService,
    WorldRequest, WorldResponse,
    compile_acl, create_world_url, evaluate_compiled_acl_with_owner,
    validate_ipfs_publish_request, publish_did_document_to_kubo,
    execute_room_actor_command,
    normalize_spoken_text, parse_capability_acl_text, parse_object_local_capability_acl,
    parse_property_command, parse_property_command_for_keys,
    Reply, Scope,
    LegacyRequirement, RequirementChecker, RequirementSet, RequirementValue,
    pin_update_add_rm, TtlCache,
    expand_tilde_path, format_system_time, is_valid_nanoid_id, parse_rfc3339_unix,
    extract_did_description_from_json, normalize_language_for_did_document,
    sender_encryption_pubkey_multibase_from_document, sender_profile_from_document,
    sender_push_endpoint_from_document,
    generate_secret_key_file, load_secret_key_bytes,
    SecureFileKind, ensure_private_dir, write_secure_file,
    IrohEndpoint,
};
use moka::sync::Cache;
use nanoid::nanoid;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, sync::{Mutex, RwLock}};
use tracing::{debug, error, info, warn};
use tracing_subscriber::prelude::*;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

mod actor;
mod avatar_commands;
mod bootstrap;
mod content_validation;
mod ingress_handlers;
mod kubo;
mod lang;
mod room;
mod schema;
mod status;
mod world_core;
mod world_protocol;
mod world_startup;

use world_protocol::*;
use world_startup::*;

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
use room::{Room, RoomAcl, RoomDispatchTask};
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
    room_url: String,
    message: String,
    ts: String,
}
#[derive(Clone, Debug, Serialize)]
struct PresenceSnapshotEvent {
    v: u8,
    kind: String,
    room: String,
    room_url: String,
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
    room_url: String,
    ts: String,
}

#[derive(Clone, Debug, Serialize)]
struct PresenceRoomStateEvent {
    v: u8,
    kind: String,
    room: String,
    room_url: String,
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
    room_url: String,
    room_title: String,
    room_description: String,
    avatars: Vec<PresenceAvatar>,
    event: RoomEvent,
    latest_event_sequence: u64,
    ts: String,
}

#[derive(Clone)]
struct WorldProtocol {
    world: Arc<World>,
    endpoint: Arc<IrohEndpoint>,
    endpoint_id: String,
    did_cache: Arc<RwLock<HashMap<String, CachedDidDocument>>>,
    push_timeout_cooldown: Arc<Mutex<HashMap<String, Instant>>>,
    service: WorldService,
}

const PUSH_TIMEOUT_COOLDOWN_SECS: u64 = 8;

#[derive(Clone, Debug)]
struct IpfsProtocol {
    kubo_url: String,
    did_cache: Arc<RwLock<HashMap<String, CachedDidDocument>>>,
}

// Locked-gate handling is now done inside WorldProtocol::accept_message:
// while the world is locked, messages are drained and a "world is locked" response
// is sent back through the sender's inbox.

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

#[derive(Clone, Debug)]
struct IncomingExitRef {
    from_room: String,
    exit_id: String,
    exit_name: String,
    to: String,
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

#[derive(Debug)]
pub struct World {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    events: Arc<RwLock<VecDeque<String>>>,
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
    /// Global avatar DID-doc registry keyed by avatar fragment.
    avatar_registry: Arc<RwLock<HashMap<String, AvatarRegistryEntry>>>,
    /// Configured TTL for room-local avatar presence state.
    avatar_presence_ttl: Arc<RwLock<Duration>>,
    /// Fast lookup from object DID to room/object inbox route.
    object_inbox_index: Cache<String, InboxRoute>,
    /// Reverse exit lookup: destination room id -> incoming exits that target it.
    exit_reverse_index: Arc<RwLock<HashMap<String, Vec<IncomingExitRef>>>>,
}

#[derive(Clone, Debug)]
struct RuntimeActorSecret {
    signing_key: [u8; 32],
}

#[derive(Clone, Debug)]
struct PresentAvatar {
    url: Did,
    room_name: String,
    handle: String,
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

fn parse_world_lang_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("lang.")?;
    let tag = rest.trim();
    if tag.is_empty() {
        return None;
    }
    Some(tag)
}

fn is_valid_world_lang_tag(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    if bytes.len() != 5 || bytes[2] != b'_' {
        return false;
    }
    bytes[0].is_ascii_lowercase()
        && bytes[1].is_ascii_lowercase()
        && bytes[3].is_ascii_uppercase()
        && bytes[4].is_ascii_uppercase()
}

#[tokio::main]
async fn main() -> Result<()> {
    world_startup::run_main().await
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
            identity: did.id(),
            owner: "did:ma:k51test".to_string(),
            agent_endpoint: "ep-1".to_string(),
            agent_services: None,
            language_order: "nb_NO:en_UK".to_string(),
            signing_secret: [0u8; 32],
            encryption_pubkey_multibase: None,
        };
        world.join_room("lobby", req, Some("pixie".to_string())).await.unwrap();

        let req2 = AvatarRequest {
            did,
            identity: "did:ma:k51test#pixie".to_string(),
            owner: "did:ma:k51test".to_string(),
            agent_endpoint: "ep-2".to_string(),
            agent_services: None,
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
            did: did.clone(),
            identity: did.id(),
            owner: "did:ma:k51stale".to_string(),
            agent_endpoint: "ep-stale".to_string(),
            agent_services: None,
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





