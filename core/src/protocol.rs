//! Shared protocol types and constants used by both the world server and home client.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ─── ALPN Protocol Identifiers ──────────────────────────────────────────────

pub const PRESENCE_ALPN: &[u8] = b"ma/presence/1";
pub const INBOX_ALPN: &[u8] = b"ma/inbox/1";
pub const AVATAR_ALPN: &[u8] = b"ma/avatar/1";
pub const IPFS_ALPN: &[u8] = b"ma/ipfs/1";
pub const DEFAULT_WORLD_RELAY_URL: &str = "https://euc1-1.relay.n0.iroh-canary.iroh.link/";

// ─── Content Types (World/Home protocol usage) ─────────────────────────────

pub const DEFAULT_CONTENT_TYPE: &str = "application/x-ma";
pub const CONTENT_TYPE_CHAT: &str = "application/x-ma-chat";
pub const CONTENT_TYPE_PRESENCE: &str = "application/x-ma-presence";
pub const CONTENT_TYPE_WORLD: &str = "application/x-ma-world";
pub const CONTENT_TYPE_EVENT: &str = "application/x-ma-event";
pub const CONTENT_TYPE_BROADCAST: &str = "application/x-ma-broadcast";
pub const CONTENT_TYPE_DOC: &str = "application/x-ma-doc";
pub const CONTENT_TYPE_WHISPER: &str = "application/x-ma-whisper";
pub const CONTENT_TYPE_MESSAGE: &str = "application/x-ma-message";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpfsPublishDidRequest {
    pub did_document_json: String,
    #[serde(default)]
    pub ipns_private_key_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_fragment: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpfsPublishDidResponse {
    pub ok: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
}

// ─── Internal Method Identifiers (object-style routing) ───────────────────

pub const ROOM_METHOD_EVENTS_POLL: &str = "room.events.poll";
pub const ROOM_METHOD_BROADCAST_SEND: &str = "room.broadcast.send";
pub const ROOM_METHOD_PRESENCE_LIST: &str = "room.presence.list";
pub const AVATAR_METHOD_ENTER: &str = "avatar.enter";
pub const AVATAR_METHOD_PING: &str = "avatar.ping";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLane {
    Inbox,
    Avatar,
}

impl WorldLane {
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Self::Inbox => INBOX_ALPN,
            Self::Avatar => AVATAR_ALPN,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Avatar => "avatar",
        }
    }

    pub fn supports_request(self, request: &WorldRequest) -> bool {
        let _ = request;
        matches!(self, Self::Inbox | Self::Avatar)
    }

    pub fn signed_content_type(self) -> Option<&'static str> {
        match self {
            Self::Inbox => Some(CONTENT_TYPE_WORLD),
            Self::Avatar => Some(CONTENT_TYPE_WORLD),
        }
    }

    pub fn supports_command(self, command: &WorldCommand) -> bool {
        match self {
            Self::Avatar => true,
            Self::Inbox => Self::is_knock_command(command),
        }
    }

    fn is_knock_command(command: &WorldCommand) -> bool {
        let WorldCommand::Message { envelope, .. } = command else {
            return false;
        };
        let crate::parser::MessageEnvelope::ActorCommand {
            target,
            command: crate::parser::ActorCommand::Raw { command },
        } = envelope else {
            return false;
        };
        if !target.eq_ignore_ascii_case("world") {
            return false;
        }
        let head = command
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        head == "knock"
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
            supports_signed: matches!(lane, WorldLane::Inbox | WorldLane::Avatar),
            supports_chat: false,
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
    #[serde(default)]
    pub identity: String,
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
        pub world_did: String,
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
    /// Enter or move avatar into a room and return authoritative room state.
    Enter {
        room_did: String,
    },
    /// Keepalive for an existing avatar; returns the current authoritative room state.
    Ping {
        room_did: String,
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

impl WorldCommand {
    pub fn internal_method(&self) -> Option<&'static str> {
        match self {
            Self::Enter { .. } => Some(AVATAR_METHOD_ENTER),
            Self::Ping { .. } => Some(AVATAR_METHOD_PING),
            Self::Message { .. } => Some(ROOM_METHOD_BROADCAST_SEND),
            Self::RoomEvents { .. } => Some(ROOM_METHOD_EVENTS_POLL),
        }
    }
}

/// Transport wrapper for requests sent over iroh connections.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorldRequest {
    pub message_cbor: Vec<u8>,
}
