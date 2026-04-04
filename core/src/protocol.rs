//! Shared protocol types and constants used by both the world server and home client.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ─── ALPN Protocol Identifiers ──────────────────────────────────────────────

pub const WORLD_ALPN: &[u8] = b"ma/world/1";
pub const CMD_ALPN: &[u8] = b"ma/cmd/1";
pub const CHAT_ALPN: &[u8] = b"ma/chat/1";
pub const CLOSET_ALPN: &[u8] = b"ma/closet/1";
pub const BROADCAST_ALPN: &[u8] = b"ma/broadcast/1";
pub const PRESENCE_ALPN: &[u8] = b"ma/presence/1";
pub const INBOX_ALPN: &[u8] = b"ma/inbox/1";
pub const WHISPER_ALPN: &[u8] = b"ma/whisper/1";
pub const DEFAULT_WORLD_RELAY_URL: &str = "https://euc1-1.relay.n0.iroh-canary.iroh.link/";

// ─── Content Types (World/Home protocol usage) ─────────────────────────────

pub const DEFAULT_CONTENT_TYPE: &str = "application/x-ma";
pub const CONTENT_TYPE_CHAT: &str = "application/x-ma-chat";
pub const CONTENT_TYPE_PRESENCE: &str = "application/x-ma-presence";
pub const CONTENT_TYPE_CMD: &str = "application/x-ma-cmd";
pub const CONTENT_TYPE_WORLD: &str = "application/x-ma-world";
pub const CONTENT_TYPE_BROADCAST: &str = "application/x-ma-broadcast";
pub const CONTENT_TYPE_DOC: &str = "application/x-ma-doc";
pub const CONTENT_TYPE_WHISPER: &str = "application/x-ma-whisper";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLane {
    World,
    Cmd,
    Chat,
}

impl WorldLane {
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Self::World => WORLD_ALPN,
            Self::Cmd => CMD_ALPN,
            Self::Chat => CHAT_ALPN,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Cmd => "cmd",
            Self::Chat => "chat",
        }
    }

    pub fn supports_request(self, request: &WorldRequest) -> bool {
        match (self, request) {
            (Self::World, WorldRequest::Signed { .. }) => true,
            (Self::Cmd, WorldRequest::Signed { .. }) => true,
            (Self::Chat, WorldRequest::Chat { .. }) => true,
            _ => false,
        }
    }

    pub fn signed_content_type(self) -> Option<&'static str> {
        match self {
            Self::World => Some(CONTENT_TYPE_WORLD),
            Self::Cmd => Some(CONTENT_TYPE_CMD),
            Self::Chat => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneCapability {
    pub lane: WorldLane,
    pub alpn: String,
    pub supports_signed: bool,
    pub supports_chat: bool,
    pub supports_whisper: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportAckCode {
    Accepted,
    UnsupportedRequestType,
    InvalidRequestJson,
    InvalidContentType,
    Rejected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransportAck {
    pub lane: String,
    pub code: TransportAckCode,
    pub detail: String,
}

impl LaneCapability {
    pub fn for_lane(lane: WorldLane) -> Self {
        Self {
            lane,
            alpn: String::from_utf8_lossy(lane.alpn()).to_string(),
            supports_signed: matches!(lane, WorldLane::World | WorldLane::Cmd),
            supports_chat: matches!(lane, WorldLane::Chat),
            supports_whisper: false,
        }
    }
}

// ─── Wire Types ─────────────────────────────────────────────────────────────

/// Avatar entry in presence snapshots and room rosters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresenceAvatar {
    pub handle: String,
    pub did: String,
}

/// A single event stored in a room's event log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomEvent {
    pub sequence: u64,
    pub room: String,
    pub kind: String,
    pub sender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_endpoint: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_cbor_b64: Option<String>,
    pub occurred_at: String,
}

/// Response from the world server to any command.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorldResponse {
    pub ok: bool,
    pub room: String,
    pub message: String,
    pub endpoint_id: String,
    pub latest_event_sequence: u64,
    pub broadcasted: bool,
    pub events: Vec<RoomEvent>,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub room_description: String,
    #[serde(default)]
    pub room_title: String,
    #[serde(default)]
    pub room_did: String,
    #[serde(default)]
    pub avatars: Vec<PresenceAvatar>,
    #[serde(default)]
    pub room_object_dids: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_ack: Option<TransportAck>,
}

/// Command sent from client to world server inside a signed message.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldCommand {
    Enter {
        room: Option<String>,
        #[serde(default)]
        preferred_handle: Option<String>,
    },
    Message {
        room: String,
        envelope: crate::parser::MessageEnvelope,
    },
    RoomEvents {
        room: String,
        since_sequence: u64,
    },
}

/// Transport wrapper for requests sent over iroh connections.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldRequest {
    Signed { message_cbor: Vec<u8> },
    Chat { room: String, message_cbor: Vec<u8> },
    Whisper { message_cbor: Vec<u8> },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClosetRequest {
    Start,
    Command {
        session_id: String,
        input: String,
    },
    HearLobby {
        session_id: String,
        since_sequence: u64,
    },
    Answer {
        session_id: String,
        field: String,
        value: String,
    },
    SubmitCitizenship {
        session_id: String,
        ipns_private_key_base64: String,
    },
    PublishDidDocument {
        session_id: String,
        did_document_json: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClosetResponse {
    pub ok: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub lobby_events: Vec<RoomEvent>,
    #[serde(default)]
    pub latest_lobby_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
}
