use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectMessageKind {
    Chat,
    Whisper,
    Emote,
    Command,
    World,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectMessageRetention {
    Durable,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectPersistencePolicy {
    Ephemeral,
    DurableDebounced,
    DurableImmediate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectProgramRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    #[serde(default)]
    pub encrypted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectReceiverListener {
    pub transport: String,
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<ObjectProgramRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ObjectMessageTarget {
    Room,
    Holder,
    Caller,
    Did(String),
    Object(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectInboxMessage {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_object: Option<String>,
    pub kind: ObjectMessageKind,
    pub body: String,
    pub sent_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_request_id: Option<String>,
    #[serde(default = "default_object_message_retention")]
    pub retention: ObjectMessageRetention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMessageIntent {
    pub target: ObjectMessageTarget,
    pub kind: ObjectMessageKind,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub attempt: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEphemeralRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub timeout_secs: u64,
    pub first_sent_at_unix: u64,
    pub last_sent_at_unix: u64,
    pub attempt: u32,
    pub intent: ObjectMessageIntent,
}

impl PendingEphemeralRequest {
    pub fn expires_at_unix(&self) -> u64 {
        self.last_sent_at_unix.saturating_add(self.timeout_secs)
    }

    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.expires_at_unix()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ObjectCommandOutput {
    InlineText(String),
    MessageIntent(ObjectMessageIntent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectCommandResult {
    #[serde(default)]
    pub outputs: Vec<ObjectCommandOutput>,
}

impl ObjectCommandResult {
    pub fn inline_text(text: impl Into<String>) -> Self {
        Self {
            outputs: vec![ObjectCommandOutput::InlineText(text.into())],
        }
    }

    pub fn from_intent(intent: ObjectMessageIntent) -> Self {
        Self {
            outputs: vec![ObjectCommandOutput::MessageIntent(intent)],
        }
    }

    pub fn push_inline_text(&mut self, text: impl Into<String>) {
        self.outputs.push(ObjectCommandOutput::InlineText(text.into()));
    }

    pub fn push_intent(&mut self, intent: ObjectMessageIntent) {
        self.outputs.push(ObjectCommandOutput::MessageIntent(intent));
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectRuntimeState {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_cid: Option<String>,
    pub room: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub receivers: Vec<ObjectReceiverListener>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_did: Option<String>,
    #[serde(default)]
    pub durable: bool,
    #[serde(default = "default_object_persistence_policy")]
    pub persistence: ObjectPersistencePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_expires_at: Option<u64>,
    #[serde(default)]
    pub inbox: VecDeque<ObjectInboxMessage>,
    #[serde(default)]
    pub pending_outbox: Vec<ObjectMessageIntent>,
    #[serde(default)]
    pub pending_ephemeral_requests: HashMap<String, PendingEphemeralRequest>,
    #[serde(default)]
    pub next_ephemeral_request_seq: u64,
    #[serde(default = "default_object_state")]
    pub state: Value,
}

fn default_object_state() -> Value {
    Value::Object(Map::new())
}

fn default_object_message_retention() -> ObjectMessageRetention {
    ObjectMessageRetention::Durable
}

fn default_object_persistence_policy() -> ObjectPersistencePolicy {
    ObjectPersistencePolicy::DurableDebounced
}

impl ObjectRuntimeState {
    pub fn intrinsic_mailbox(room: &str) -> Self {
        Self {
            id: "mailbox".to_string(),
            name: "mailbox".to_string(),
            kind: "receiver".to_string(),
            definition_cid: None,
            room: room.to_string(),
            aliases: vec!["mailbox".to_string(), "messaging-device".to_string()],
            receivers: vec![ObjectReceiverListener {
                transport: "iroh".to_string(),
                protocol: "ma/inbox/1".to_string(),
                role: Some("world-inbox".to_string()),
                address: None,
            }],
            owner_did: None,
            durable: true,
            persistence: ObjectPersistencePolicy::DurableImmediate,
            ttl_secs: None,
            holder: None,
            opened_by: None,
            locked_by: None,
            lock_expires_at: None,
            inbox: VecDeque::new(),
            pending_outbox: Vec::new(),
            pending_ephemeral_requests: HashMap::new(),
            next_ephemeral_request_seq: 0,
            state: default_object_state(),
        }
    }

    fn allocate_request_id(&mut self, now_unix: u64) -> String {
        self.next_ephemeral_request_seq = self.next_ephemeral_request_seq.saturating_add(1);
        format!("{}-{}-{}", self.id, now_unix, self.next_ephemeral_request_seq)
    }

    pub fn clear_expired_lock(&mut self, now_secs: u64) {
        let expired = self
            .lock_expires_at
            .map(|expiry| expiry <= now_secs)
            .unwrap_or(false);
        if expired {
            self.opened_by = None;
            self.locked_by = None;
            self.lock_expires_at = None;
        }
    }

    pub fn matches_target(&self, target: &str) -> bool {
        let token = target.trim().to_ascii_lowercase();
        token == self.id.to_ascii_lowercase()
            || token == self.name.to_ascii_lowercase()
            || self
                .aliases
                .iter()
                .any(|alias| alias.trim().eq_ignore_ascii_case(token.as_str()))
    }

    pub fn has_receiver_role(&self, role: &str) -> bool {
        let needle = role.trim();
        if needle.is_empty() {
            return false;
        }
        self.receivers
            .iter()
            .any(|listener| listener.role.as_deref().map(|value| value.eq_ignore_ascii_case(needle)).unwrap_or(false))
    }

    pub fn has_receiver_protocol(&self, protocol: &str) -> bool {
        let needle = protocol.trim();
        if needle.is_empty() {
            return false;
        }
        self.receivers
            .iter()
            .any(|listener| listener.protocol.eq_ignore_ascii_case(needle))
    }

    pub fn push_inbox_message(&mut self, message: ObjectInboxMessage, max_items: usize) {
        self.inbox.push_back(message);
        if self.inbox.len() > max_items {
            let overflow = self.inbox.len() - max_items;
            self.inbox.drain(..overflow);
        }
    }

    pub fn push_ephemeral_inbox_message(
        &mut self,
        mut message: ObjectInboxMessage,
        max_items: usize,
    ) {
        message.retention = ObjectMessageRetention::Ephemeral;
        self.push_inbox_message(message, max_items);
    }

    pub fn push_durable_inbox_message(
        &mut self,
        mut message: ObjectInboxMessage,
        max_items: usize,
    ) {
        message.retention = ObjectMessageRetention::Durable;
        self.push_inbox_message(message, max_items);
    }

    pub fn pop_inbox_message(&mut self) -> Option<ObjectInboxMessage> {
        self.inbox.pop_front()
    }

    pub fn queue_outbound_intent(&mut self, intent: ObjectMessageIntent) {
        self.pending_outbox.push(intent);
    }

    pub fn begin_ephemeral_request(
        &mut self,
        mut intent: ObjectMessageIntent,
        now_unix: u64,
        default_timeout_secs: u64,
    ) -> String {
        let request_id = intent
            .request_id
            .clone()
            .unwrap_or_else(|| self.allocate_request_id(now_unix));
        let timeout_secs = intent.timeout_secs.unwrap_or(default_timeout_secs).max(1);
        let attempt = intent.attempt.max(1);

        intent.request_id = Some(request_id.clone());
        intent.timeout_secs = Some(timeout_secs);
        intent.attempt = attempt;

        self.queue_outbound_intent(intent.clone());
        self.pending_ephemeral_requests.insert(
            request_id.clone(),
            PendingEphemeralRequest {
                request_id: request_id.clone(),
                session_id: intent.session_id.clone(),
                timeout_secs,
                first_sent_at_unix: now_unix,
                last_sent_at_unix: now_unix,
                attempt,
                intent,
            },
        );

        request_id
    }

    pub fn retry_ephemeral_request(&mut self, request_id: &str, now_unix: u64) -> Option<u32> {
        let is_expired = self
            .pending_ephemeral_requests
            .get(request_id)
            .map(|tracker| tracker.is_expired(now_unix))
            .unwrap_or(false);
        if is_expired {
            self.pending_ephemeral_requests.remove(request_id);
            return None;
        }

        let (attempt, intent) = {
            let tracker = self.pending_ephemeral_requests.get_mut(request_id)?;
            tracker.attempt = tracker.attempt.saturating_add(1);
            tracker.last_sent_at_unix = now_unix;
            tracker.intent.attempt = tracker.attempt;
            tracker.intent.request_id = Some(tracker.request_id.clone());
            tracker.intent.timeout_secs = Some(tracker.timeout_secs);
            (tracker.attempt, tracker.intent.clone())
        };

        self.queue_outbound_intent(intent);
        Some(attempt)
    }

    pub fn resolve_ephemeral_reply(&mut self, reply: &ObjectInboxMessage) -> bool {
        let Some(request_id) = reply.reply_to_request_id.as_deref() else {
            return false;
        };
        self.pending_ephemeral_requests.remove(request_id).is_some()
    }

    pub fn has_pending_ephemeral_request(&self, request_id: &str) -> bool {
        self.pending_ephemeral_requests.contains_key(request_id)
    }

    pub fn reap_expired_ephemeral_requests(&mut self, now_unix: u64) -> Vec<String> {
        let mut expired = Vec::new();
        self.pending_ephemeral_requests.retain(|id, tracker| {
            let keep = !tracker.is_expired(now_unix);
            if !keep {
                expired.push(id.clone());
            }
            keep
        });
        expired
    }

    pub fn drain_outbound_intents(&mut self) -> Vec<ObjectMessageIntent> {
        self.pending_outbox.drain(..).collect()
    }

    pub fn durable_inbox_len(&self) -> usize {
        self.inbox
            .iter()
            .filter(|msg| msg.retention == ObjectMessageRetention::Durable)
            .count()
    }

    pub fn ephemeral_inbox_len(&self) -> usize {
        self.inbox
            .iter()
            .filter(|msg| msg.retention == ObjectMessageRetention::Ephemeral)
            .count()
    }

    pub fn persisted_snapshot(&self) -> Self {
        let mut cloned = self.clone();
        cloned
            .inbox
            .retain(|msg| msg.retention == ObjectMessageRetention::Durable);
        cloned.pending_outbox.clear();
        cloned.pending_ephemeral_requests.clear();
        cloned.next_ephemeral_request_seq = 0;
        cloned
    }
}