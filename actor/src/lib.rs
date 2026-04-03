use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use bip39::{Language, Mnemonic};
use blake3;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    Key, XChaCha20Poly1305, XNonce,
};
use did_ma::{
    Did, Document, EncryptionKey, Message, SigningKey, VerificationMethod,
};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use js_sys;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

fn recipient_inbox_endpoint_id(document: &Document) -> Result<String, JsValue> {
    if let Some(ma) = document.ma.as_ref() {
        if let Some(endpoint) = core_resolve_inbox_endpoint_id(
            ma.current_inbox.as_deref(),
            ma.presence_hint.as_deref(),
            ma.transports.as_ref(),
        ) {
            return Ok(endpoint);
        }
    }

    Err(js_err("recipient DID document has no usable inbox transport endpoint"))
}

async fn send_whisper_signed_message(target_endpoint_id: &str, message_cbor: Vec<u8>) -> Result<InboxResponse, JsValue> {
    let target: EndpointId = target_endpoint_id
        .trim()
        .parse()
        .map_err(|e| js_err(format!("invalid recipient endpoint id: {e}")))?;

    let endpoint = Endpoint::builder(presets::N0)
        .bind()
        .await
        .map_err(|e| js_err(format!("sender endpoint bind failed: {e}")))?;
    let _ = endpoint.online().await;

    let relay_source = core_normalize_relay_url(DEFAULT_WORLD_RELAY_URL);
    let relay_url: RelayUrl = relay_source
        .parse()
        .map_err(|e| js_err(format!("relay URL parse failed for '{}': {}", relay_source, e)))?;

    let endpoint_addr = EndpointAddr::new(target).with_relay_url(relay_url);
    let connection = endpoint
        .connect(endpoint_addr, WHISPER_ALPN)
        .await
        .map_err(|e| js_err(format!("whisper endpoint.connect() failed: {}", e)))?;

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| js_err(format!("whisper connection.open_bi() failed: {}", e)))?;

    let request = InboxRequest::Signed { message_cbor };
    let payload = serde_json::to_vec(&request).map_err(js_err)?;

    send.write_u32(payload.len() as u32).await.map_err(js_err)?;
    send.write_all(&payload).await.map_err(js_err)?;
    send.flush().await.map_err(js_err)?;

    let frame_len = recv.read_u32().await.map_err(js_err)? as usize;
    if frame_len > 256 * 1024 {
        return Err(js_err(format!("whisper response frame too large: {}", frame_len)));
    }
    let mut bytes = vec![0u8; frame_len];
    recv.read_exact(&mut bytes).await.map_err(js_err)?;

    let _ = send.finish();
    connection.close(0u32.into(), b"ok");
    endpoint.close().await;

    serde_json::from_slice::<InboxResponse>(&bytes).map_err(js_err)
}

struct WorldConnCache {
    endpoint: Endpoint,
    connection: Connection,
    send_stream: SendStream,
    recv_stream: RecvStream,
    target_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorldTransportKind {
    World,
    Cmd,
    Chat,
}

impl WorldTransportKind {
    fn alpn(self) -> &'static [u8] {
        match self {
            WorldTransportKind::World => WORLD_ALPN,
            WorldTransportKind::Cmd => CMD_ALPN,
            WorldTransportKind::Chat => CHAT_ALPN,
        }
    }
}

thread_local! {
    static WORLD_CONN_CACHE: RefCell<Option<WorldConnCache>> = RefCell::new(None);
    static CMD_CONN_CACHE: RefCell<Option<WorldConnCache>> = RefCell::new(None);
    static CHAT_CONN_CACHE: RefCell<Option<WorldConnCache>> = RefCell::new(None);
    static ACL_COMPILED_CACHE: RefCell<HashMap<String, CompiledCapabilityAcl>> = RefCell::new(HashMap::new());
    static ROOM_DID_CACHE: RefCell<HashMap<String, CachedDidEntry>> = RefCell::new(HashMap::new());
    static ROOM_OBJECT_DID_CACHE: RefCell<HashMap<String, CachedDidEntry>> = RefCell::new(HashMap::new());
    static ACTIVE_ROOM_CACHE: RefCell<Option<String>> = RefCell::new(None);
}

#[derive(Clone, Debug)]
struct CachedDidEntry {
    did: String,
    expires_at_ms: f64,
}

const ROOM_DID_CACHE_TTL_MS: f64 = 5.0 * 60.0 * 1000.0;

fn take_conn_cache(kind: WorldTransportKind) -> Option<WorldConnCache> {
    match kind {
        WorldTransportKind::World => WORLD_CONN_CACHE.with(|c| c.borrow_mut().take()),
        WorldTransportKind::Cmd => CMD_CONN_CACHE.with(|c| c.borrow_mut().take()),
        WorldTransportKind::Chat => CHAT_CONN_CACHE.with(|c| c.borrow_mut().take()),
    }
}

fn store_conn_cache(kind: WorldTransportKind, cache: WorldConnCache) {
    match kind {
        WorldTransportKind::World => WORLD_CONN_CACHE.with(|c| *c.borrow_mut() = Some(cache)),
        WorldTransportKind::Cmd => CMD_CONN_CACHE.with(|c| *c.borrow_mut() = Some(cache)),
        WorldTransportKind::Chat => CHAT_CONN_CACHE.with(|c| *c.borrow_mut() = Some(cache)),
    }
}

fn clear_conn_cache(kind: WorldTransportKind) {
    match kind {
        WorldTransportKind::World => WORLD_CONN_CACHE.with(|c| *c.borrow_mut() = None),
        WorldTransportKind::Cmd => CMD_CONN_CACHE.with(|c| *c.borrow_mut() = None),
        WorldTransportKind::Chat => CHAT_CONN_CACHE.with(|c| *c.borrow_mut() = None),
    }
}

fn with_inbox_state<T>(f: impl FnOnce(&Option<InboxListenerState>) -> T) -> T {
    INBOX_STATE.with(|slot| {
        let state_ref = slot.borrow();
        f(&state_ref)
    })
}

fn set_inbox_state(state: InboxListenerState) {
    INBOX_STATE.with(|slot| {
        *slot.borrow_mut() = Some(state);
    });
}

impl ProtocolHandler for InboxProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let from_endpoint = connection.remote_id().to_string();
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
                    format!("inbox frame too large: {}", frame_len),
                )));
            }

            let mut bytes = vec![0u8; frame_len];
            recv.read_exact(&mut bytes).await.map_err(AcceptError::from_err)?;

            let response = match serde_json::from_slice::<InboxRequest>(&bytes) {
                Ok(InboxRequest::Signed { message_cbor }) => {
                    if let Some(expected) = self.expected_content_type {
                        match Message::from_cbor(&message_cbor) {
                            Ok(message) if message.content_type == expected => {
                                let item = InboxMessage {
                                    message_cbor_b64: B64.encode(message_cbor),
                                    from_endpoint: from_endpoint.clone(),
                                    received_at: now_unix_secs(),
                                };
                                let mut queue = self.queue.write().await;
                                if queue.len() >= MAX_INBOX_EVENTS {
                                    queue.pop_front();
                                }
                                queue.push_back(item);
                                InboxResponse {
                                    ok: true,
                                    message: "queued".to_string(),
                                }
                            }
                            Ok(message) => InboxResponse {
                                ok: false,
                                message: format!(
                                    "{} expects content_type={} but got {}",
                                    String::from_utf8_lossy(self.lane_label),
                                    expected,
                                    message.content_type
                                ),
                            },
                            Err(err) => InboxResponse {
                                ok: false,
                                message: format!(
                                    "invalid signed message on {}: {}",
                                    String::from_utf8_lossy(self.lane_label),
                                    err
                                ),
                            },
                        }
                    } else {
                        let item = InboxMessage {
                            message_cbor_b64: B64.encode(message_cbor),
                            from_endpoint: from_endpoint.clone(),
                            received_at: now_unix_secs(),
                        };
                        let mut queue = self.queue.write().await;
                        if queue.len() >= MAX_INBOX_EVENTS {
                            queue.pop_front();
                        }
                        queue.push_back(item);
                        InboxResponse {
                            ok: true,
                            message: "queued".to_string(),
                        }
                    }
                }
                Err(err) => InboxResponse {
                    ok: false,
                    message: format!("invalid inbox request JSON: {}", err),
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

async fn ensure_inbox_listener_with_secret(secret_key: SecretKey) -> Result<String, JsValue> {
    if let Some(existing_id) = with_inbox_state(|state| state.as_ref().map(|s| s.endpoint.id().to_string())) {
        return Ok(existing_id);
    }

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .bind()
        .await
        .map_err(|e| js_err(format!("inbox endpoint bind failed: {e}")))?;

    let queue = Arc::new(RwLock::new(VecDeque::with_capacity(MAX_INBOX_EVENTS)));
    let inbox_protocol = InboxProtocol {
        queue: queue.clone(),
        expected_content_type: None,
        lane_label: INBOX_ALPN,
    };
    let whisper_protocol = InboxProtocol {
        queue: queue.clone(),
        expected_content_type: Some(CONTENT_TYPE_WHISPER),
        lane_label: WHISPER_ALPN,
    };
    let broadcast_protocol = InboxProtocol {
        queue: queue.clone(),
        expected_content_type: Some(CONTENT_TYPE_BROADCAST),
        lane_label: BROADCAST_ALPN,
    };
    let presence_protocol = InboxProtocol {
        queue: queue.clone(),
        expected_content_type: Some(CONTENT_TYPE_PRESENCE),
        lane_label: PRESENCE_ALPN,
    };

    let router = Router::builder(endpoint.clone())
        .accept(INBOX_ALPN, inbox_protocol)
        .accept(WHISPER_ALPN, whisper_protocol)
        .accept(BROADCAST_ALPN, broadcast_protocol)
        .accept(PRESENCE_ALPN, presence_protocol)
        .spawn();

    let endpoint_id = endpoint.id().to_string();
    set_inbox_state(InboxListenerState {
        endpoint,
        router,
        queue,
    });

    Ok(endpoint_id)
}

async fn create_stream_cache(
    target_id_str: &str,
    relay_hint: Option<&str>,
    kind: WorldTransportKind,
) -> Result<WorldConnCache, JsValue> {
    let target: EndpointId = target_id_str
        .trim()
        .parse()
        .map_err(|e| js_err(format!("invalid endpoint id: {e}")))?;

    let endpoint = Endpoint::builder(presets::N0).bind()
        .await
        .map_err(js_err)?;

    // Give the endpoint a brief chance to establish its relay/discovery presence
    // before attempting peer connect by endpoint id.
    let _ = endpoint.online().await;

    let mut endpoint_addr = EndpointAddr::new(target);
    let relay_source = core_normalize_relay_url(relay_hint.unwrap_or(DEFAULT_WORLD_RELAY_URL));
    
    match relay_source.parse::<RelayUrl>() {
        Ok(relay_url) => {
            endpoint_addr = endpoint_addr.with_relay_url(relay_url);
        }
        Err(e) => {
            return Err(js_err(format!("relay URL parse failed for '{}': {}", relay_source, e)));
        }
    }

    let connection = endpoint.connect(endpoint_addr, kind.alpn())
        .await
        .map_err(|e| js_err(format!("endpoint.connect() failed: {}", e)))?;
    
    let (send_stream, recv_stream) = connection.open_bi()
        .await
        .map_err(|e| js_err(format!("connection.open_bi() failed: {}", e)))?;

    Ok(WorldConnCache {
        endpoint,
        connection,
        send_stream,
        recv_stream,
        target_id: target_id_str.to_string(),
    })
}

async fn get_or_create_stream_cache(
    target_id_str: &str,
    kind: WorldTransportKind,
) -> Result<WorldConnCache, JsValue> {
    if let Some(cached) = take_conn_cache(kind) {
        if cached.target_id == target_id_str {
            return Ok(cached);
        }
        cached.connection.close(0u32.into(), b"switch target");
        cached.endpoint.close().await;
    }

    create_stream_cache(target_id_str, None, kind).await
}

async fn exchange_on_stream(cache: &mut WorldConnCache, request: &WorldRequest) -> Result<WorldResponse, JsValue> {
    let payload = serde_json::to_vec(request).map_err(js_err)?;
    if payload.len() > 256 * 1024 {
        return Err(js_err("request frame too large"));
    }

    cache
        .send_stream
        .write_u32(payload.len() as u32)
        .await
        .map_err(|e| js_err(format!("write frame length failed: {e}")))?;
    cache
        .send_stream
        .write_all(&payload)
        .await
        .map_err(|e| js_err(format!("iroh send failed: {e}")))?;
    cache
        .send_stream
        .flush()
        .await
        .map_err(|e| js_err(format!("iroh flush failed: {e}")))?;

    let response_len = cache
        .recv_stream
        .read_u32()
        .await
        .map_err(|e| js_err(format!("read frame length failed: {e}")))? as usize;
    if response_len > 512 * 1024 {
        return Err(js_err("response frame too large"));
    }

    let mut response_bytes = vec![0u8; response_len];
    cache
        .recv_stream
        .read_exact(&mut response_bytes)
        .await
        .map_err(|e| js_err(format!("iroh read failed: {e}")))?;

    serde_json::from_slice(&response_bytes).map_err(js_err)
}
use ma_core::{
    CompiledCapabilityAcl,
    compile_acl,
    CONTENT_TYPE_BROADCAST, CONTENT_TYPE_CHAT, CONTENT_TYPE_CMD, CONTENT_TYPE_PRESENCE,
    CONTENT_TYPE_WORLD, CONTENT_TYPE_WHISPER,
    DEFAULT_WORLD_RELAY_URL,
    evaluate_compiled_acl_with_owner,
    did_root as core_did_root,
    find_alias_for_address as core_find_alias_for_address,
    find_did_by_endpoint as core_find_did_by_endpoint,
    humanize_identifier as core_humanize_identifier,
    humanize_text as core_humanize_text,
    normalize_endpoint_id as core_normalize_endpoint_id,
    normalize_relay_url as core_normalize_relay_url,
    parse_capability_acl_text,
    parse_message,
    resolve_inbox_endpoint_id as core_resolve_inbox_endpoint_id,
    resolve_alias_input as core_resolve_alias_input,
    RoomEvent, WorldCommand, WorldRequest, WorldResponse,
    BROADCAST_ALPN, CHAT_ALPN, CMD_ALPN, INBOX_ALPN, PRESENCE_ALPN, WHISPER_ALPN, WORLD_ALPN,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

// ── Data structures ────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct EncryptedIdentityBundle {
    version: u32,
    kdf: String,
    salt_b64: String,
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Serialize, Deserialize)]
struct IdentityBundlePlain {
    version: u32,
    created_at: u64,
    ipns: String,
    signing_private_key_hex: String,
    encryption_private_key_hex: String,
    #[serde(default)]
    iroh_secret_key_hex: Option<String>,
    document: Document,
}

#[derive(Serialize)]
struct CreateResult {
    encrypted_bundle: String,
    did: String,
    ipns: String,
    document_json: String,
}

#[derive(Serialize)]
struct UnlockResult {
    did: String,
    ipns: String,
    document_json: String,
}

#[derive(Serialize)]
struct UpdateResult {
    encrypted_bundle: String,
    did: String,
    ipns: String,
    document_json: String,
}

/// Client-side result wrapper that extends the shared WorldResponse with
/// fields populated locally (e.g. pending whispers from the inbox).
#[derive(Serialize)]
struct WorldActionResult {
    #[serde(flatten)]
    response: WorldResponse,
    #[serde(default)]
    pending_whispers: Vec<RoomEvent>,
}

const WORLD_TARGET_DID: &str = "did:ma:world";
const MAX_INBOX_EVENTS: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InboxMessage {
    message_cbor_b64: String,
    from_endpoint: String,
    received_at: u64,
}

#[derive(Serialize)]
struct InboxPollResult {
    endpoint_id: String,
    messages: Vec<InboxMessage>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InboxRequest {
    Signed { message_cbor: Vec<u8> },
}

#[derive(Serialize, Deserialize)]
struct InboxResponse {
    ok: bool,
    message: String,
}

#[derive(Clone, Debug)]
struct InboxProtocol {
    queue: Arc<RwLock<VecDeque<InboxMessage>>>,
    expected_content_type: Option<&'static str>,
    lane_label: &'static [u8],
}

#[derive(Debug)]
struct InboxListenerState {
    endpoint: Endpoint,
    router: Router,
    queue: Arc<RwLock<VecDeque<InboxMessage>>>,
}

thread_local! {
    static INBOX_STATE: RefCell<Option<InboxListenerState>> = RefCell::new(None);
}

#[derive(Serialize)]
struct IpnsPointer {
    version: u32,
    identity_bundle_cid: String,
    current_host_hint: String,
    updated_at: u64,
    sequence: u64,
}

#[derive(Serialize)]
struct ActorDidCacheEntryDebug {
    key: String,
    did: String,
    expires_at_ms: u64,
    ttl_remaining_ms: u64,
}

#[derive(Serialize)]
struct ActorDidCacheDebug {
    now_ms: u64,
    ttl_config_ms: u64,
    active_room: Option<String>,
    room_dids: Vec<ActorDidCacheEntryDebug>,
    room_object_dids: Vec<ActorDidCacheEntryDebug>,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn js_err(msg: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&msg.to_string())
}

fn compiled_acl_from_text_cached(source: &str, acl_text: &str) -> Result<CompiledCapabilityAcl, JsValue> {
    let cache_key = format!("{}:{}", source, blake3::hash(acl_text.as_bytes()).to_hex());

    if let Some(cached) = ACL_COMPILED_CACHE.with(|slot| slot.borrow().get(&cache_key).cloned()) {
        return Ok(cached);
    }

    let acl = parse_capability_acl_text(acl_text, source).map_err(js_err)?;
    let compiled = compile_acl(&acl, source).map_err(js_err)?;

    ACL_COMPILED_CACHE.with(|slot| {
        slot.borrow_mut().insert(cache_key, compiled.clone());
    });

    Ok(compiled)
}

#[wasm_bindgen]
pub fn evaluate_capability_acl(
    subject_did: &str,
    capability: &str,
    global_acl_text: &str,
    local_acl_text: &str,
    world_owner_did: &str,
    local_owner_did: &str,
) -> Result<bool, JsValue> {
    let subject = subject_did.trim();
    let cap = capability.trim();
    if subject.is_empty() {
        return Err(js_err("subject_did cannot be empty"));
    }
    if cap.is_empty() {
        return Err(js_err("capability cannot be empty"));
    }

    let world_owner = if world_owner_did.trim().is_empty() {
        None
    } else {
        Some(world_owner_did.trim())
    };
    let local_owner = if local_owner_did.trim().is_empty() {
        None
    } else {
        Some(local_owner_did.trim())
    };

    let global_match = if global_acl_text.trim().is_empty() {
        true
    } else {
        let compiled = compiled_acl_from_text_cached("actor-global-acl", global_acl_text)?;
        evaluate_compiled_acl_with_owner(&compiled, subject, world_owner, cap)
    };

    if !global_match {
        return Ok(false);
    }

    let local_match = if local_acl_text.trim().is_empty() {
        true
    } else {
        let compiled = compiled_acl_from_text_cached("actor-local-acl", local_acl_text)?;
        evaluate_compiled_acl_with_owner(&compiled, subject, local_owner, cap)
    };

    Ok(local_match)
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn generate_ipns_id() -> Result<String, String> {
    // Produces a k51-style identifier (alphanumeric, 59 chars) compatible with Did::new
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let rand = random_bytes::<56>()?;
    let suffix: String = rand.iter().map(|b| CHARS[(*b as usize) % 36] as char).collect();
    Ok(format!("k51{suffix}"))
}

fn now_unix_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn clamp_ms_u64(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else if value >= u64::MAX as f64 {
        u64::MAX
    } else {
        value as u64
    }
}

fn normalize_phrase_text(input: &str) -> String {
    input
        .split_whitespace()
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_string_map(map_json: &str) -> Result<HashMap<String, String>, JsValue> {
    let trimmed = map_json.trim();
    if trimmed.is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str::<HashMap<String, String>>(trimmed)
        .map_err(|e| js_err(format!("invalid map JSON: {e}")))
}

fn room_object_cache_key(room: &str, object_token: &str) -> String {
    let room_key = room.trim().to_ascii_lowercase();
    let token_key = object_token
        .trim()
        .trim_start_matches('@')
        .to_ascii_lowercase();
    format!("{}\n{}", room_key, token_key)
}

fn clear_room_cache_for(room: &str) {
    let room_key = room.trim().to_ascii_lowercase();
    if room_key.is_empty() {
        return;
    }

    ROOM_DID_CACHE.with(|slot| {
        slot.borrow_mut().remove(&room_key);
    });

    let room_prefix = format!("{}\n", room_key);
    ROOM_OBJECT_DID_CACHE.with(|slot| {
        slot.borrow_mut().retain(|key, _| !key.starts_with(&room_prefix));
    });
}

fn clear_all_room_did_caches() {
    ROOM_DID_CACHE.with(|slot| slot.borrow_mut().clear());
    ROOM_OBJECT_DID_CACHE.with(|slot| slot.borrow_mut().clear());
    ACTIVE_ROOM_CACHE.with(|slot| *slot.borrow_mut() = None);
}

fn switch_active_room_cache(room: &str) {
    let room_key = room.trim().to_ascii_lowercase();
    if room_key.is_empty() {
        return;
    }
    ACTIVE_ROOM_CACHE.with(|slot| {
        let mut active = slot.borrow_mut();
        if let Some(previous) = active.as_ref() {
            if previous != &room_key {
                clear_room_cache_for(previous);
            }
        }
        *active = Some(room_key);
    });
}

fn cache_room_did(room: &str, room_did: &str) {
    let room = room.trim();
    let room_did = room_did.trim();
    if room.is_empty() || room_did.is_empty() {
        return;
    }
    let expires_at_ms = now_ms() + ROOM_DID_CACHE_TTL_MS;
    ROOM_DID_CACHE.with(|slot| {
        slot.borrow_mut().insert(
            room.to_ascii_lowercase(),
            CachedDidEntry {
                did: room_did.to_string(),
                expires_at_ms,
            },
        );
    });
}

fn cache_room_object_did(room: &str, object_token: &str, object_did: &str) {
    let key = room_object_cache_key(room, object_token);
    if key.trim().is_empty() || object_did.trim().is_empty() {
        return;
    }
    let expires_at_ms = now_ms() + ROOM_DID_CACHE_TTL_MS;
    ROOM_OBJECT_DID_CACHE.with(|slot| {
        slot.borrow_mut().insert(
            key,
            CachedDidEntry {
                did: object_did.trim().to_string(),
                expires_at_ms,
            },
        );
    });
}

fn lookup_room_object_did(room: &str, object_token: &str) -> Option<String> {
    let key = room_object_cache_key(room, object_token);
    let now = now_ms();
    ROOM_OBJECT_DID_CACHE.with(|slot| {
        let mut map = slot.borrow_mut();
        let Some(entry) = map.get(&key).cloned() else {
            return None;
        };
        if entry.expires_at_ms <= now {
            map.remove(&key);
            return None;
        }
        Some(entry.did)
    })
}

fn update_room_did_cache_from_response(response: &WorldResponse) {
    if response.room.is_empty() || response.room_did.is_empty() {
        return;
    }
    switch_active_room_cache(&response.room);
    if Did::validate(&response.room_did).is_ok() {
        cache_room_did(&response.room, &response.room_did);
    }
    for (token, did_text) in &response.room_object_dids {
        if let Ok(did) = Did::try_from(did_text.as_str()) {
            if did.fragment.is_some() {
                cache_room_object_did(&response.room, token, &did.id());
            }
        }
    }
}

fn normalize_use_alias_command(room: &str, text: &str) -> String {
    let trimmed = text.trim();
    let (prefix, rest) = if let Some(rest) = trimmed.strip_prefix("use ") {
        ("use ", rest)
    } else if let Some(rest) = trimmed.strip_prefix("/use ") {
        ("/use ", rest)
    } else {
        return text.to_string();
    };

    let Some((target_raw, alias_raw)) = rest.split_once(" as ") else {
        return text.to_string();
    };
    let target = target_raw.trim();
    let alias = alias_raw.trim();
    if target.is_empty() || !alias.starts_with('@') {
        return text.to_string();
    }

    if let Ok(target_did) = Did::try_from(target) {
        if let Some(object_id) = target_did.fragment.as_ref() {
            cache_room_object_did(room, object_id, &target_did.id());
        }
        return text.to_string();
    }

    if let Some(cached_did) = lookup_room_object_did(room, target) {
        return format!("{}{} as {}", prefix, cached_did, alias);
    }

    text.to_string()
}

fn cache_entry_debug(key: String, entry: &CachedDidEntry, now: f64) -> ActorDidCacheEntryDebug {
    let remaining = if entry.expires_at_ms <= now {
        0.0
    } else {
        entry.expires_at_ms - now
    };
    ActorDidCacheEntryDebug {
        key,
        did: entry.did.clone(),
        expires_at_ms: clamp_ms_u64(entry.expires_at_ms),
        ttl_remaining_ms: clamp_ms_u64(remaining),
    }
}

// ── Crypto ─────────────────────────────────────────────────────────────────────

fn derive_key_argon2(password: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(19456, 2, 1, Some(32)).map_err(|e| format!("argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = [0u8; 32];
    argon2
        .hash_password_into(password, salt, &mut output)
        .map_err(|e| format!("argon2: {e}"))?;
    Ok(output)
}

fn encrypt_bundle(passphrase: &str, plaintext: &[u8]) -> Result<EncryptedIdentityBundle, String> {
    let salt = random_bytes::<16>()?;
    let nonce_bytes = random_bytes::<24>()?;
    let key_bytes = derive_key_argon2(passphrase.as_bytes(), &salt)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|e| e.to_string())?;
    Ok(EncryptedIdentityBundle {
        version: 1,
        kdf: "argon2id".to_string(),
        salt_b64: B64.encode(salt),
        nonce_b64: B64.encode(nonce_bytes),
        ciphertext_b64: B64.encode(ciphertext),
    })
}

fn decrypt_bundle(passphrase: &str, bundle: &EncryptedIdentityBundle) -> Result<Vec<u8>, String> {
    let salt = B64.decode(&bundle.salt_b64).map_err(|e| e.to_string())?;
    let nonce_bytes = B64.decode(&bundle.nonce_b64).map_err(|e| e.to_string())?;
    let ciphertext = B64.decode(&bundle.ciphertext_b64).map_err(|e| e.to_string())?;
    let key_bytes = derive_key_argon2(passphrase.as_bytes(), &salt)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(&nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext.as_slice())
        .map_err(|_| "wrong passphrase or corrupted bundle".to_string())
}

async fn send_world_request(endpoint_id: &str, request: WorldRequest) -> Result<WorldResponse, JsValue> {
    let mut last_error: Option<JsValue> = None;
    for _ in 0..2 {
        let mut cache = get_or_create_stream_cache(endpoint_id, WorldTransportKind::World).await?;
        match exchange_on_stream(&mut cache, &request).await {
            Ok(response) => {
                store_conn_cache(WorldTransportKind::World, cache);
                return Ok(response);
            }
            Err(err) => {
                last_error = Some(err);
                cache.connection.close(0u32.into(), b"stream error");
                cache.endpoint.close().await;
                clear_conn_cache(WorldTransportKind::World);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| js_err("world request failed")))
}

async fn send_world_cmd_request(endpoint_id: &str, request: WorldRequest) -> Result<WorldResponse, JsValue> {
    let mut last_error: Option<JsValue> = None;
    for _ in 0..2 {
        let mut cache = get_or_create_stream_cache(endpoint_id, WorldTransportKind::Cmd).await?;
        match exchange_on_stream(&mut cache, &request).await {
            Ok(response) => {
                store_conn_cache(WorldTransportKind::Cmd, cache);
                return Ok(response);
            }
            Err(err) => {
                last_error = Some(err);
                cache.connection.close(0u32.into(), b"stream error");
                cache.endpoint.close().await;
                clear_conn_cache(WorldTransportKind::Cmd);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| js_err("world cmd request failed")))
}

async fn send_world_chat_request(endpoint_id: &str, request: WorldRequest) -> Result<WorldResponse, JsValue> {
    let mut last_error: Option<JsValue> = None;
    for _ in 0..2 {
        let mut cache = get_or_create_stream_cache(endpoint_id, WorldTransportKind::Chat).await?;
        match exchange_on_stream(&mut cache, &request).await {
            Ok(response) => {
                store_conn_cache(WorldTransportKind::Chat, cache);
                return Ok(response);
            }
            Err(err) => {
                last_error = Some(err);
                cache.connection.close(0u32.into(), b"stream error");
                cache.endpoint.close().await;
                clear_conn_cache(WorldTransportKind::Chat);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| js_err("world chat request failed")))
}

/// Close and drop the cached world connection (call on lock/logout).
#[wasm_bindgen]
pub async fn disconnect_world() {
    if let Some(cached) = take_conn_cache(WorldTransportKind::World) {
        cached.connection.close(0u32.into(), b"bye");
        cached.endpoint.close().await;
    }

    if let Some(cached) = take_conn_cache(WorldTransportKind::Cmd) {
        cached.connection.close(0u32.into(), b"bye");
        cached.endpoint.close().await;
    }

    if let Some(cached) = take_conn_cache(WorldTransportKind::Chat) {
        cached.connection.close(0u32.into(), b"bye");
        cached.endpoint.close().await;
    }

    let state = INBOX_STATE.with(|slot| slot.borrow_mut().take());
    if let Some(listener) = state {
        let _ = listener.router.shutdown().await;
        listener.endpoint.close().await;
    }

    clear_all_room_did_caches();
}

/// Ensure a direct inbox listener is running for this ma-actor session.
/// Returns local inbox endpoint id.
#[wasm_bindgen]
pub async fn start_inbox_listener(
    passphrase: &str,
    encrypted_bundle_json: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;

    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let key_hex = plain
        .iroh_secret_key_hex
        .as_deref()
        .ok_or_else(|| js_err("bundle missing iroh secret key; run ensure_bundle_iroh_secret first"))?;
    let iroh_secret_key = restore_iroh_secret_key(key_hex)?;

    ensure_inbox_listener_with_secret(iroh_secret_key).await
}

/// Ensure the encrypted bundle has a persisted iroh secret key.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn ensure_bundle_iroh_secret(
    passphrase: &str,
    encrypted_bundle_json: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;

    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let mut plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let needs_key = plain
        .iroh_secret_key_hex
        .as_deref()
        .map(|v| v.trim().is_empty())
        .unwrap_or(true);
    if needs_key {
        let generated = SecretKey::from_bytes(&random_bytes::<32>().map_err(js_err)?);
        plain.iroh_secret_key_hex = Some(hex::encode(generated.to_bytes()));
    }

    let document_json = plain.document.marshal().map_err(js_err)?;
    let plain_json = serde_json::to_string(&plain).map_err(js_err)?;
    let encrypted = encrypt_bundle(passphrase, plain_json.as_bytes()).map_err(js_err)?;

    let result = UpdateResult {
        encrypted_bundle: serde_json::to_string(&encrypted).map_err(js_err)?,
        did: plain.document.id.clone(),
        ipns: plain.ipns,
        document_json,
    };

    serde_json::to_string(&result).map_err(js_err)
}

/// Rotate (replace) the persisted iroh secret key in the encrypted bundle.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn rotate_bundle_iroh_secret(
    passphrase: &str,
    encrypted_bundle_json: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;

    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let mut plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let generated = SecretKey::from_bytes(&random_bytes::<32>().map_err(js_err)?);
    plain.iroh_secret_key_hex = Some(hex::encode(generated.to_bytes()));

    let document_json = plain.document.marshal().map_err(js_err)?;
    let plain_json = serde_json::to_string(&plain).map_err(js_err)?;
    let encrypted = encrypt_bundle(passphrase, plain_json.as_bytes()).map_err(js_err)?;

    let result = UpdateResult {
        encrypted_bundle: serde_json::to_string(&encrypted).map_err(js_err)?,
        did: plain.document.id.clone(),
        ipns: plain.ipns,
        document_json,
    };

    serde_json::to_string(&result).map_err(js_err)
}

/// Poll and drain direct inbox messages received over iroh.
#[wasm_bindgen]
pub async fn poll_inbox_messages() -> Result<String, JsValue> {
    let Some((endpoint_id, queue)) = with_inbox_state(|state| {
        state
            .as_ref()
            .map(|s| (s.endpoint.id().to_string(), s.queue.clone()))
    }) else {
        return Ok(serde_json::to_string(&InboxPollResult {
            endpoint_id: String::new(),
            messages: Vec::new(),
        })
        .map_err(js_err)?);
    };

    let mut guard = queue.write().await;
    let messages = guard.drain(..).collect::<Vec<_>>();
    drop(guard);

    serde_json::to_string(&InboxPollResult {
        endpoint_id,
        messages,
    })
    .map_err(js_err)
}

/// Inspect a signed message CBOR (base64) and return minimal metadata.
#[wasm_bindgen]
pub fn inspect_signed_message(message_cbor_b64: &str) -> Result<String, JsValue> {
    let cbor = B64.decode(message_cbor_b64).map_err(js_err)?;
    let message = Message::from_cbor(&cbor).map_err(js_err)?;

    #[derive(Serialize)]
    struct MessageMeta {
        from: String,
        to: String,
        content_type: String,
        content_text: String,
    }

    serde_json::to_string(&MessageMeta {
        from: message.from,
        to: message.to,
        content_type: message.content_type,
        content_text: String::from_utf8_lossy(&message.content).to_string(),
    })
    .map_err(js_err)
}

#[wasm_bindgen]
pub fn alias_did_root(input: &str) -> String {
    core_did_root(input)
}

#[wasm_bindgen]
pub fn alias_normalize_endpoint_id(input: &str) -> String {
    core_normalize_endpoint_id(input).unwrap_or_default()
}

#[wasm_bindgen]
pub fn alias_resolve_input(input: &str, alias_book_json: &str) -> Result<String, JsValue> {
    let alias_book = parse_string_map(alias_book_json)?;
    Ok(core_resolve_alias_input(input, &alias_book))
}

#[wasm_bindgen]
pub fn alias_find_alias_for_address(address: &str, alias_book_json: &str) -> Result<String, JsValue> {
    let alias_book = parse_string_map(alias_book_json)?;
    Ok(core_find_alias_for_address(address, &alias_book).unwrap_or_default())
}

#[wasm_bindgen]
pub fn alias_find_did_by_endpoint(
    endpoint_like: &str,
    did_endpoint_map_json: &str,
) -> Result<String, JsValue> {
    let did_endpoint_map = parse_string_map(did_endpoint_map_json)?;
    Ok(core_find_did_by_endpoint(endpoint_like, &did_endpoint_map).unwrap_or_default())
}

#[wasm_bindgen]
pub fn alias_humanize_identifier(value: &str, alias_book_json: &str) -> Result<String, JsValue> {
    let alias_book = parse_string_map(alias_book_json)?;
    Ok(core_humanize_identifier(value, &alias_book))
}

#[wasm_bindgen]
pub fn alias_humanize_text(text: &str, alias_book_json: &str) -> Result<String, JsValue> {
    let alias_book = parse_string_map(alias_book_json)?;
    Ok(core_humanize_text(text, &alias_book))
}

#[wasm_bindgen]
pub fn actor_cache_room_object_did(room: &str, object_token: &str, object_did: &str) -> Result<(), JsValue> {
    let did = Did::try_from(object_did).map_err(js_err)?;
    if did.fragment.is_none() {
        return Err(js_err("object DID must include #fragment"));
    }
    cache_room_object_did(room, object_token, &did.id());
    Ok(())
}

#[wasm_bindgen]
pub fn actor_cache_room_did(room: &str, room_did: &str) -> Result<(), JsValue> {
    Did::try_from(room_did).map_err(js_err)?;
    cache_room_did(room, room_did);
    Ok(())
}

#[wasm_bindgen]
pub fn actor_debug_room_did_cache() -> Result<String, JsValue> {
    let now = now_ms();

    let room_dids = ROOM_DID_CACHE.with(|slot| {
        let mut map = slot.borrow_mut();
        map.retain(|_, entry| entry.expires_at_ms > now);
        let mut rows = map
            .iter()
            .map(|(k, v)| cache_entry_debug(k.clone(), v, now))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        rows
    });

    let room_object_dids = ROOM_OBJECT_DID_CACHE.with(|slot| {
        let mut map = slot.borrow_mut();
        map.retain(|_, entry| entry.expires_at_ms > now);
        let mut rows = map
            .iter()
            .map(|(k, v)| cache_entry_debug(k.clone(), v, now))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        rows
    });

    let active_room = ACTIVE_ROOM_CACHE.with(|slot| slot.borrow().clone());

    serde_json::to_string_pretty(&ActorDidCacheDebug {
        now_ms: clamp_ms_u64(now),
        ttl_config_ms: clamp_ms_u64(ROOM_DID_CACHE_TTL_MS),
        active_room,
        room_dids,
        room_object_dids,
    })
    .map_err(js_err)
}

fn build_signed_world_request(
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    command: WorldCommand,
    content_type: &str,
    timestamp_ms: u64,
) -> Result<WorldRequest, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;
    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let actor_name = actor_name.trim();
    if actor_name.is_empty() {
        return Err(js_err("actor_name is required for DID fragment"));
    }

    let from_did = Did::try_from(plain.document.id.as_str())
        .and_then(|did| did.with_fragment(actor_name))
        .map_err(js_err)?;

    let target_world_did = plain
        .document
        .ma
        .as_ref()
        .and_then(|ma| ma.world.clone())
        .filter(|did| Did::validate(did).is_ok())
        .unwrap_or_else(|| WORLD_TARGET_DID.to_string());

    let signing_key = restore_signing_key(&plain.ipns, &plain.signing_private_key_hex)?;
    let content = serde_json::to_vec(&command).map_err(js_err)?;
    
    // Create message with custom timestamp from JavaScript (in milliseconds, convert to seconds)
    let message = build_signed_message_with_js_time(
        from_did.id().to_string(),
        target_world_did,
        content_type.to_string(),
        content,
        &signing_key,
        timestamp_ms,
    )?;

    Ok(WorldRequest::Signed {
        message_cbor: message.to_cbor().map_err(js_err)?,
    })
}

fn build_signed_message_with_js_time(
    from: String,
    to: String,
    content_type: String,
    content: Vec<u8>,
    signing_key: &SigningKey,
    timestamp_ms: u64,
) -> Result<Message, JsValue> {
    let timestamp_secs = timestamp_ms / 1000;
    let mut message = Message {
        id: timestamp_ms.to_string(),
        message_type: did_ma::msg::message_type(),
        from,
        to,
        created_at: timestamp_secs,
        content_type,
        reply_to: None,
        content,
        signature: Vec::new(),
    };
    message
        .sign(signing_key)
        .map_err(|e| js_err(format!("message signing failed: {}", e)))?;
    Ok(message)
}

fn restore_signing_key(ipns: &str, private_key_hex: &str) -> Result<SigningKey, JsValue> {
    let sign_did = Did::new(ipns, "sig").map_err(js_err)?;
    let private_key_vec = hex::decode(private_key_hex).map_err(js_err)?;
    let private_key: [u8; 32] = private_key_vec
        .try_into()
        .map_err(|_| js_err("invalid signing private key length"))?;

    SigningKey::from_private_key_bytes(sign_did, private_key).map_err(js_err)
}

fn restore_encryption_key(ipns: &str, private_key_hex: &str) -> Result<EncryptionKey, JsValue> {
    let enc_did = Did::new(ipns, "enc").map_err(js_err)?;
    let private_key_vec = hex::decode(private_key_hex).map_err(js_err)?;
    let private_key: [u8; 32] = private_key_vec
        .try_into()
        .map_err(|_| js_err("invalid encryption private key length"))?;

    EncryptionKey::from_private_key_bytes(enc_did, private_key).map_err(js_err)
}

fn restore_iroh_secret_key(private_key_hex: &str) -> Result<SecretKey, JsValue> {
    let private_key_vec = hex::decode(private_key_hex).map_err(js_err)?;
    let private_key: [u8; 32] = private_key_vec
        .try_into()
        .map_err(|_| js_err("invalid iroh secret key length"))?;
    Ok(SecretKey::from_bytes(&private_key))
}

fn parse_signed_message_with_sender_document(sender_document_json: &str, message_cbor_b64: &str) -> Result<Message, JsValue> {
    let sender_document = Document::unmarshal(sender_document_json).map_err(js_err)?;
    let message_cbor = B64.decode(message_cbor_b64).map_err(js_err)?;
    let message = Message::from_cbor(&message_cbor).map_err(js_err)?;
    message.verify_with_document(&sender_document).map_err(js_err)?;
    Ok(message)
}

fn derive_whisper_key(shared_secret: [u8; 32]) -> [u8; 32] {
    let hash = blake3::Hasher::new()
        .update(b"ma-whisper-content")
        .update(&shared_secret)
        .finalize();
    *hash.as_bytes()
}

fn encrypt_whisper_content(plaintext: &[u8], sender_encryption_key: &EncryptionKey, recipient_document: &Document) -> Result<Vec<u8>, JsValue> {
    let recipient_pub = X25519PublicKey::from(recipient_document.key_agreement_public_key_bytes().map_err(js_err)?);
    let shared_secret = sender_encryption_key.shared_secret(&recipient_pub);
    let key_bytes = derive_whisper_key(shared_secret);

    let nonce_bytes = random_bytes::<24>().map_err(js_err)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|e| js_err(format!("whisper encryption failed: {e}")))?;

    // payload layout: nonce(24) || ciphertext
    let mut out = Vec::with_capacity(24 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt_whisper_payload(cipher_payload: &[u8], recipient_encryption_key: &EncryptionKey, sender_document: &Document) -> Result<Vec<u8>, JsValue> {
    if cipher_payload.len() < 24 {
        return Err(js_err("invalid whisper payload"));
    }
    let (nonce_bytes, ciphertext) = cipher_payload.split_at(24);

    let sender_pub = X25519PublicKey::from(sender_document.key_agreement_public_key_bytes().map_err(js_err)?);
    let recipient_secret = StaticSecret::from(recipient_encryption_key.private_key_bytes());
    let shared_secret = recipient_secret.diffie_hellman(&sender_pub).to_bytes();
    let key_bytes = derive_whisper_key(shared_secret);

    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| js_err("failed to decrypt whisper payload"))
}

fn update_bundle_document<F>(
    passphrase: &str,
    encrypted_bundle_json: &str,
    update: F,
) -> Result<String, JsValue>
where
    F: FnOnce(&mut Document) -> Result<(), JsValue>,
{
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;

    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let mut plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    update(&mut plain.document)?;

    let signing_key = restore_signing_key(&plain.ipns, &plain.signing_private_key_hex)?;
    let assertion_method = plain
        .document
        .get_verification_method_by_id(&plain.document.assertion_method)
        .map_err(js_err)?
        .clone();
    plain.document.sign(&signing_key, &assertion_method).map_err(js_err)?;

    let document_json = plain.document.marshal().map_err(js_err)?;
    let plain_json = serde_json::to_string(&plain).map_err(js_err)?;
    let encrypted = encrypt_bundle(passphrase, plain_json.as_bytes()).map_err(js_err)?;

    let result = UpdateResult {
        encrypted_bundle: serde_json::to_string(&encrypted).map_err(js_err)?,
        did: plain.document.id.clone(),
        ipns: plain.ipns,
        document_json,
    };

    serde_json::to_string(&result).map_err(js_err)
}

// ── Exported WASM functions ────────────────────────────────────────────────────

fn create_identity_internal(passphrase: &str, ipns: &str) -> Result<String, JsValue> {
    let root_did = Did::new_root(ipns).map_err(js_err)?;
    let sign_did = Did::new(ipns, "sig").map_err(js_err)?;
    let enc_did = Did::new(ipns, "enc").map_err(js_err)?;

    let signing_key = SigningKey::generate(sign_did).map_err(js_err)?;
    let encryption_key = EncryptionKey::generate(enc_did).map_err(js_err)?;
    let iroh_secret_key = SecretKey::from_bytes(&random_bytes::<32>().map_err(js_err)?);

    let mut document = Document::new(&root_did, &root_did);

    let assertion_vm = VerificationMethod::new(
        root_did.base_id(),
        root_did.base_id(),
        signing_key.key_type.clone(),
        "sig",
        signing_key.public_key_multibase.clone(),
    )
    .map_err(js_err)?;

    let key_agreement_vm = VerificationMethod::new(
        root_did.base_id(),
        root_did.base_id(),
        encryption_key.key_type.clone(),
        "enc",
        encryption_key.public_key_multibase.clone(),
    )
    .map_err(js_err)?;

    let assertion_vm_id = assertion_vm.id.clone();
    document.add_verification_method(assertion_vm.clone()).map_err(js_err)?;
    document.add_verification_method(key_agreement_vm.clone()).map_err(js_err)?;
    document.assertion_method = assertion_vm_id;
    document.key_agreement = key_agreement_vm.id.clone();
    document.set_ma_type("agent");
    document.sign(&signing_key, &assertion_vm).map_err(js_err)?;

    let plain = IdentityBundlePlain {
        version: 1,
        created_at: now_unix_secs(),
        ipns: ipns.to_string(),
        signing_private_key_hex: hex::encode(signing_key.private_key_bytes()),
        encryption_private_key_hex: hex::encode(encryption_key.private_key_bytes()),
        iroh_secret_key_hex: Some(hex::encode(iroh_secret_key.to_bytes())),
        document,
    };

    let document_json = plain.document.marshal().map_err(js_err)?;
    let plain_json = serde_json::to_string(&plain).map_err(js_err)?;
    let encrypted = encrypt_bundle(passphrase, plain_json.as_bytes()).map_err(js_err)?;

    let result = CreateResult {
        encrypted_bundle: serde_json::to_string(&encrypted).map_err(js_err)?,
        did: root_did.id(),
        ipns: ipns.to_string(),
        document_json,
    };

    serde_json::to_string(&result).map_err(js_err)
}

/// Generate a new identity, encrypt the bundle with `passphrase`.
/// Returns JSON: `{ encrypted_bundle, did, ipns }`
#[wasm_bindgen]
pub fn create_identity(passphrase: &str) -> Result<String, JsValue> {
    let ipns = generate_ipns_id().map_err(js_err)?;
    create_identity_internal(passphrase, &ipns)
}

/// Generate a new identity bound to an existing IPNS identifier from Kubo.
/// Use this when you already have a Kubo key and want DID/IPNS to match exactly.
#[wasm_bindgen]
pub fn create_identity_with_ipns(passphrase: &str, ipns: &str) -> Result<String, JsValue> {
    let ipns = ipns.trim();
    if ipns.is_empty() {
        return Err(js_err("ipns is required"));
    }
    create_identity_internal(passphrase, ipns)
}

/// Decrypt an encrypted bundle with `passphrase`.
/// Returns JSON: `{ did, ipns, document_json }`
#[wasm_bindgen]
pub fn unlock_identity(passphrase: &str, encrypted_bundle_json: &str) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;

    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;

    let plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let result = UnlockResult {
        did: plain.document.id.clone(),
        ipns: plain.ipns.clone(),
        document_json: plain.document.marshal().map_err(js_err)?,
    };

    serde_json::to_string(&result).map_err(js_err)
}

/// Update the optional `ma:presenceHint` field in the DID document and re-sign it.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn set_bundle_presence_hint(
    passphrase: &str,
    encrypted_bundle_json: &str,
    hint: &str,
) -> Result<String, JsValue> {
    update_bundle_document(passphrase, encrypted_bundle_json, |document| {
        document.set_presence_hint(hint).map_err(js_err)
    })
}

/// Update optional `ma:lang` and `ma:language` fields in the DID document and re-sign it.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn set_bundle_language_preferences(
    passphrase: &str,
    encrypted_bundle_json: &str,
    lang: &str,
    language: &str,
) -> Result<String, JsValue> {
    update_bundle_document(passphrase, encrypted_bundle_json, |document| {
        document.set_lang(lang).map_err(js_err)?;
        document.set_language(language).map_err(js_err)
    })
}

/// Update the optional `ma:world` field in the DID document and re-sign it.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn set_bundle_world(
    passphrase: &str,
    encrypted_bundle_json: &str,
    world_did: &str,
) -> Result<String, JsValue> {
    update_bundle_document(passphrase, encrypted_bundle_json, |document| {
        document.set_ma_world(world_did);
        Ok(())
    })
}

/// Update the `ma:transports` field in the DID document with the agent's live
/// iroh inbox endpoint and re-sign it.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn set_bundle_transports(
    passphrase: &str,
    encrypted_bundle_json: &str,
    endpoint_id: &str,
) -> Result<String, JsValue> {
    let inbox_hint = format!("/ma-iroh/{}/ma/inbox/1", endpoint_id);
    let whisper_hint = format!("/ma-iroh/{}/ma/whisper/1", endpoint_id);
    let broadcast_hint = format!("/ma-iroh/{}/ma/broadcast/1", endpoint_id);
    let transports = serde_json::json!([inbox_hint.clone(), whisper_hint, broadcast_hint]);
    update_bundle_document(passphrase, encrypted_bundle_json, move |document| {
        document.set_ma_transports(transports);
        document.set_ma_current_inbox(&inbox_hint);
        document.set_presence_hint(&inbox_hint).map_err(js_err)?;
        Ok(())
    })
}

/// Remove optional `ma:lang` and `ma:language` fields from the DID document and re-sign it.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn clear_bundle_language_preferences(
    passphrase: &str,
    encrypted_bundle_json: &str,
) -> Result<String, JsValue> {
    update_bundle_document(passphrase, encrypted_bundle_json, |document| {
        document.clear_lang();
        document.clear_language();
        Ok(())
    })
}

/// Remove the optional `ma:presenceHint` field from the DID document and re-sign it.
/// Returns JSON: `{ encrypted_bundle, did, ipns, document_json }`
#[wasm_bindgen]
pub fn clear_bundle_presence_hint(
    passphrase: &str,
    encrypted_bundle_json: &str,
) -> Result<String, JsValue> {
    update_bundle_document(passphrase, encrypted_bundle_json, |document| {
        document.clear_presence_hint();
        Ok(())
    })
}

/// Enter a world over iroh using the world protocol.
#[wasm_bindgen]
pub async fn connect_world(endpoint_id: &str) -> Result<(), JsValue> {
    let cache = get_or_create_stream_cache(endpoint_id, WorldTransportKind::World).await?;
    store_conn_cache(WorldTransportKind::World, cache);
    Ok(())
}

#[wasm_bindgen]
pub async fn connect_world_with_relay(endpoint_id: &str, relay_url: &str) -> Result<(), JsValue> {
    let cache = create_stream_cache(endpoint_id, Some(relay_url), WorldTransportKind::World).await?;
    store_conn_cache(WorldTransportKind::World, cache);
    Ok(())
}

/// Enter a world over iroh using the world protocol.
#[wasm_bindgen]
pub async fn enter_world(
    endpoint_id: &str,
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    room: &str,
) -> Result<String, JsValue> {
    let room = room.trim();
    let timestamp_ms = js_sys::Date::now() as u64;
    let request = build_signed_world_request(
        passphrase,
        encrypted_bundle_json,
        actor_name,
        WorldCommand::Enter {
            room: if room.is_empty() {
                None
            } else {
                Some(room.to_string())
            },
            preferred_handle: Some(actor_name.trim().to_string()),
        },
        CONTENT_TYPE_WORLD,
        timestamp_ms,
    )?;
    let response = send_world_request(endpoint_id, request).await?;
    update_room_did_cache_from_response(&response);

    serde_json::to_string(&WorldActionResult {
        response,
        pending_whispers: vec![],
    })
    .map_err(js_err)
}

/// Send a signed `application/x-ma-chat` message to a room.
#[wasm_bindgen]
pub async fn send_world_chat(
    endpoint_id: &str,
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    room: &str,
    text: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;
    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let actor_name = actor_name.trim();
    let from_did = Did::try_from(plain.document.id.as_str())
        .and_then(|did| did.with_fragment(actor_name))
        .map_err(js_err)?;
    let signing_key = restore_signing_key(&plain.ipns, &plain.signing_private_key_hex)?;

    let timestamp_ms = js_sys::Date::now() as u64;
    let message = build_signed_message_with_js_time(
        from_did.id(),
        String::new(),
        CONTENT_TYPE_CHAT.to_string(),
        text.as_bytes().to_vec(),
        &signing_key,
        timestamp_ms,
    )?;

    let request = WorldRequest::Chat {
        room: room.trim().to_string(),
        message_cbor: message.to_cbor().map_err(js_err)?,
    };
    let response = send_world_chat_request(endpoint_id, request).await?;
    update_room_did_cache_from_response(&response);

    serde_json::to_string(&WorldActionResult {
        response,
        pending_whispers: vec![],
    })
    .map_err(js_err)
}

/// Send an E2E-encrypted `application/x-ma-whisper` to recipient DID.
#[wasm_bindgen]
pub async fn send_world_whisper(
    _endpoint_id: &str,
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    recipient_document_json: &str,
    text: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;
    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let recipient_document = Document::unmarshal(recipient_document_json).map_err(js_err)?;
    let recipient_endpoint_id = recipient_inbox_endpoint_id(&recipient_document)?;
    let actor_name = actor_name.trim();
    let from_did = Did::try_from(plain.document.id.as_str())
        .and_then(|did| did.with_fragment(actor_name))
        .map_err(js_err)?;

    let signing_key = restore_signing_key(&plain.ipns, &plain.signing_private_key_hex)?;
    let encryption_key = restore_encryption_key(&plain.ipns, &plain.encryption_private_key_hex)?;
    let cipher_payload = encrypt_whisper_content(text.as_bytes(), &encryption_key, &recipient_document)?;

    let timestamp_ms = js_sys::Date::now() as u64;
    let message = build_signed_message_with_js_time(
        from_did.id(),
        recipient_document.id.clone(),
        CONTENT_TYPE_WHISPER.to_string(),
        cipher_payload,
        &signing_key,
        timestamp_ms,
    )?;

    let response = send_whisper_signed_message(&recipient_endpoint_id, message.to_cbor().map_err(js_err)?).await?;

    serde_json::to_string(&WorldActionResult {
        response: WorldResponse {
            ok: response.ok,
            room: String::new(),
            message: response.message,
            endpoint_id: recipient_endpoint_id,
            latest_event_sequence: 0,
            broadcasted: false,
            events: Vec::new(),
            handle: String::new(),
            room_description: String::new(),
            room_title: String::new(),
            room_did: String::new(),
            avatars: Vec::new(),
            room_object_dids: HashMap::new(),
            transport_ack: None,
        },
        pending_whispers: Vec::new(),
    })
    .map_err(js_err)
}

/// Decode a base64 CBOR message and return the plaintext chat content.
#[wasm_bindgen]
pub fn decode_chat_event_message(
    sender_document_json: &str,
    message_cbor_b64: &str,
) -> Result<String, JsValue> {
    let message = parse_signed_message_with_sender_document(sender_document_json, message_cbor_b64)?;
    if message.content_type != CONTENT_TYPE_CHAT {
        return Err(js_err(format!("expected application/x-ma-chat, got {}", message.content_type)));
    }
    String::from_utf8(message.content).map_err(js_err)
}

/// Decode a base64 CBOR whisper message and decrypt its content for current identity.
#[wasm_bindgen]
pub fn decode_whisper_event_message(
    passphrase: &str,
    encrypted_bundle_json: &str,
    sender_document_json: &str,
    message_cbor_b64: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedIdentityBundle = serde_json::from_str(encrypted_bundle_json)
        .map_err(|e| js_err(format!("invalid bundle JSON: {e}")))?;
    let plain_bytes = decrypt_bundle(passphrase, &encrypted).map_err(js_err)?;
    let plain: IdentityBundlePlain = serde_json::from_slice(&plain_bytes)
        .map_err(|e| js_err(format!("bundle corrupted: {e}")))?;

    let message = parse_signed_message_with_sender_document(sender_document_json, message_cbor_b64)?;
    if message.content_type != CONTENT_TYPE_WHISPER {
        return Err(js_err(format!("expected application/x-ma-whisper, got {}", message.content_type)));
    }

    let recipient_encryption_key = restore_encryption_key(&plain.ipns, &plain.encryption_private_key_hex)?;
    let sender_document = Document::unmarshal(sender_document_json).map_err(js_err)?;
    let plaintext = decrypt_whisper_payload(&message.content, &recipient_encryption_key, &sender_document)?;
    String::from_utf8(plaintext).map_err(js_err)
}

/// Send a room message over iroh using the world protocol.
#[wasm_bindgen]
pub async fn send_world_message(
    endpoint_id: &str,
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    room: &str,
    _language: &str,
    text: &str,
) -> Result<String, JsValue> {
    let timestamp_ms = js_sys::Date::now() as u64;
    let envelope = parse_message(text);
    let is_admin_world_command =
        matches!(&envelope, ma_core::MessageEnvelope::ActorCommand { target, .. } if target.eq_ignore_ascii_case("world"));
    if !is_admin_world_command {
        return Err(js_err("ma/world/1 only accepts @@ world commands; send normal commands over ma/cmd/1"));
    }
    let request = build_signed_world_request(
        passphrase,
        encrypted_bundle_json,
        actor_name,
        WorldCommand::Message {
            room: room.trim().to_string(),
            envelope,
        },
        CONTENT_TYPE_WORLD,
        timestamp_ms,
    )?;
    let response = send_world_request(endpoint_id, request).await?;
    update_room_did_cache_from_response(&response);

    serde_json::to_string(&WorldActionResult {
        response,
        pending_whispers: vec![],
    })
    .map_err(js_err)
}

/// Send a room/gameplay command over iroh using the command protocol.
#[wasm_bindgen]
pub async fn send_world_cmd(
    endpoint_id: &str,
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    room: &str,
    _language: &str,
    text: &str,
) -> Result<String, JsValue> {
    let timestamp_ms = js_sys::Date::now() as u64;
    let rewritten_text = normalize_use_alias_command(room, text);
    let envelope = parse_message(&rewritten_text);
    let is_admin_world_command =
        matches!(&envelope, ma_core::MessageEnvelope::ActorCommand { target, .. } if target.eq_ignore_ascii_case("world"));
    if is_admin_world_command {
        return Err(js_err("@@ world commands must use ma/world/1"));
    }

    let request = build_signed_world_request(
        passphrase,
        encrypted_bundle_json,
        actor_name,
        WorldCommand::Message {
            room: room.trim().to_string(),
            envelope,
        },
        CONTENT_TYPE_CMD,
        timestamp_ms,
    )?;
    let response = send_world_cmd_request(endpoint_id, request).await?;
    update_room_did_cache_from_response(&response);

    serde_json::to_string(&WorldActionResult {
        response,
        pending_whispers: vec![],
    })
    .map_err(js_err)
}

/// Poll room events over iroh using the world protocol.
#[wasm_bindgen]
pub async fn poll_world_events(
    endpoint_id: &str,
    passphrase: &str,
    encrypted_bundle_json: &str,
    actor_name: &str,
    room: &str,
    since_sequence: u64,
) -> Result<String, JsValue> {
    let timestamp_ms = js_sys::Date::now() as u64;
    let request = build_signed_world_request(
        passphrase,
        encrypted_bundle_json,
        actor_name,
        WorldCommand::RoomEvents {
            room: room.trim().to_string(),
            since_sequence,
        },
        CONTENT_TYPE_WORLD,
        timestamp_ms,
    )?;
    let response = send_world_request(endpoint_id, request).await?;
    update_room_did_cache_from_response(&response);

    serde_json::to_string(&WorldActionResult {
        response,
        pending_whispers: vec![],
    })
    .map_err(js_err)
}

/// Build an IPNS pointer record (JSON) for publishing via Kubo or w3s.
/// `sequence` should be the last published sequence; this increments it.
/// Returns pretty-printed JSON of the pointer record.
#[wasm_bindgen]
pub fn build_ipns_pointer(
    ipns: &str,
    bundle_cid: &str,
    host_hint: &str,
    sequence: u32,
) -> Result<String, JsValue> {
    if bundle_cid.is_empty() {
        return Err(js_err("bundle CID is required"));
    }
    let _ = ipns; // included for caller clarity; IPNS name is the key, not stored in value
    let pointer = IpnsPointer {
        version: 1,
        identity_bundle_cid: bundle_cid.to_string(),
        current_host_hint: host_hint.to_string(),
        updated_at: now_unix_secs(),
        sequence: sequence as u64 + 1,
    };
    serde_json::to_string_pretty(&pointer).map_err(js_err)
}

/// Generate a standard BIP39 English mnemonic phrase.
/// Supported word counts: 12, 15, 18, 21, 24.
#[wasm_bindgen]
pub fn generate_bip39_phrase(word_count: u8) -> Result<String, JsValue> {
    let entropy_len = match word_count {
        12 => 16,
        15 => 20,
        18 => 24,
        21 => 28,
        24 => 32,
        _ => return Err(js_err("word_count must be one of 12, 15, 18, 21, 24")),
    };

    let mut entropy = vec![0u8; entropy_len];
    getrandom::getrandom(&mut entropy).map_err(js_err)?;
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy).map_err(js_err)?;
    Ok(mnemonic.to_string())
}

/// Normalize and validate a BIP39 English mnemonic phrase.
/// Returns the normalized phrase if valid.
#[wasm_bindgen]
pub fn normalize_bip39_phrase(phrase: &str) -> Result<String, JsValue> {
    let normalized = normalize_phrase_text(phrase);
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &normalized).map_err(js_err)?;
    Ok(mnemonic.to_string())
}
