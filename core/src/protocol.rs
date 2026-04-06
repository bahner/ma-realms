//! Shared protocol types and constants used by both the world server and home client.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ─── ALPN Protocol Identifiers ──────────────────────────────────────────────

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
pub const CONTENT_TYPE_WORLD: &str = "application/x-ma-world";
pub const CONTENT_TYPE_BROADCAST: &str = "application/x-ma-broadcast";
pub const CONTENT_TYPE_DOC: &str = "application/x-ma-doc";
pub const CONTENT_TYPE_WHISPER: &str = "application/x-ma-whisper";

// ─── Internal Method Identifiers (object-style routing) ───────────────────

pub const ROOM_METHOD_EVENTS_POLL: &str = "room.events.poll";
pub const ROOM_METHOD_BROADCAST_SEND: &str = "room.broadcast.send";
pub const ROOM_METHOD_PRESENCE_LIST: &str = "room.presence.list";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLane {
    Inbox,
}

impl WorldLane {
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Self::Inbox => INBOX_ALPN,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
        }
    }

    pub fn supports_request(self, request: &WorldRequest) -> bool {
        let _ = request;
        matches!(self, Self::Inbox)
    }

    pub fn signed_content_type(self) -> Option<&'static str> {
        match self {
            Self::Inbox => Some(CONTENT_TYPE_WORLD),
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
            supports_signed: matches!(lane, WorldLane::Inbox),
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

impl WorldCommand {
    pub fn internal_method(&self) -> Option<&'static str> {
        match self {
            Self::Message { .. } => Some(ROOM_METHOD_BROADCAST_SEND),
            Self::RoomEvents { .. } => Some(ROOM_METHOD_EVENTS_POLL),
            Self::Enter { .. } => None,
        }
    }
}

/// Transport wrapper for requests sent over iroh connections.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorldRequest {
    pub message_cbor: Vec<u8>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desired_fragment: Option<String>,
    },
    PublishDidDocument {
        session_id: String,
        did_document_json: String,
        ipns_private_key_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desired_fragment: Option<String>,
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

impl ClosetResponse {
    fn base(session_id: &str, ok: bool, message: impl Into<String>) -> Self {
        Self {
            ok,
            message: message.into(),
            session_id: Some(session_id.to_string()),
            prompt: None,
            lobby_events: Vec::new(),
            latest_lobby_sequence: 0,
            did: None,
            fragment: None,
            key_name: None,
        }
    }

    pub fn ok(session_id: &str, message: impl Into<String>) -> Self {
        Self::base(session_id, true, message)
    }

    pub fn ok_unscoped(message: impl Into<String>) -> Self {
        let mut response = Self::base("_", true, message);
        response.session_id = None;
        response
    }

    pub fn err(session_id: &str, message: impl Into<String>) -> Self {
        Self::base(session_id, false, message)
    }

    pub fn err_unscoped(message: impl Into<String>) -> Self {
        let mut response = Self::base("_", false, message);
        response.session_id = None;
        response
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    pub fn with_lobby_events(mut self, events: Vec<RoomEvent>, latest_sequence: u64) -> Self {
        self.lobby_events = events;
        self.latest_lobby_sequence = latest_sequence;
        self
    }

    pub fn with_latest_lobby_sequence(mut self, latest_sequence: u64) -> Self {
        self.latest_lobby_sequence = latest_sequence;
        self
    }

    pub fn with_did(mut self, did: impl Into<String>) -> Self {
        self.did = Some(did.into());
        self
    }

    pub fn with_fragment(mut self, fragment: impl Into<String>) -> Self {
        self.fragment = Some(fragment.into());
        self
    }

    pub fn with_key_name(mut self, key_name: impl Into<String>) -> Self {
        self.key_name = Some(key_name.into());
        self
    }

    pub fn with_did_opt(mut self, did: Option<String>) -> Self {
        self.did = did;
        self
    }

    pub fn with_fragment_opt(mut self, fragment: Option<String>) -> Self {
        self.fragment = fragment;
        self
    }

    pub fn with_session_id_opt(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::ClosetResponse;

    #[test]
    fn scoped_builder_sets_session_and_flags() {
        let response = ClosetResponse::ok("sess-1", "ready");
        assert!(response.ok);
        assert_eq!(response.session_id.as_deref(), Some("sess-1"));
        assert_eq!(response.message, "ready");
    }

    #[test]
    fn unscoped_builder_omits_session() {
        let response = ClosetResponse::err_unscoped("boom");
        assert!(!response.ok);
        assert!(response.session_id.is_none());
        assert_eq!(response.message, "boom");
    }
}
