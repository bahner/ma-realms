use crate::actor::Avatar;
use ma_core::{ExitData, RoomEvent, TtlCache};
use std::{collections::{HashMap, HashSet, VecDeque}, time::Duration};

const DEFAULT_AVATAR_STATE_TTL_SECS: u64 = 30;

/// Access control list for a room.
/// Evaluation order: deny list → allow list (with * wildcard).
/// Owner access is enforced through ACL membership (allow/deny), not bypass rules.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RoomAcl {
    /// Root DID of the room owner.
    pub owner: Option<String>,
    /// Allow list of root DIDs, supports '*' wildcard for open access.
    pub allow: HashSet<String>,
    /// Explicit deny list of root DIDs. Takes priority over allow list.
    pub deny: HashSet<String>,
}

impl RoomAcl {
    /// Open room (lobby default): everyone welcome, no owner.
    pub fn open() -> Self {
        let mut allow = HashSet::new();
        allow.insert("*".to_string());
        Self {
            owner: None,
            allow,
            deny: HashSet::new(),
        }
    }

    /// Returns `true` if `identity` is allowed to enter this room.
    pub fn can_enter(&self, identity: &str) -> bool {
        // Explicit deny takes priority.
        if self.deny.contains(identity) {
            return false;
        }
        // Wildcard allow or explicit allow.
        self.allow.contains("*") || self.allow.contains(identity)
    }

    /// Human-readable summary for status / log display.
    pub fn summary(&self) -> String {
        if self.allow.contains("*") && self.deny.is_empty() {
            return "*".to_string();
        }
        let mut parts = Vec::new();
        if self.allow.contains("*") {
            parts.push("*".to_string());
        }
        for d in self.allow.iter().filter(|v| v.as_str() != "*") {
            parts.push(d.clone());
        }
        for d in &self.deny {
            parts.push(format!("!{d}"));
        }
        parts.join(", ")
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub name: String,
    #[serde(default)]
    pub titles: HashMap<String, String>,
    #[serde(skip)]
    pub avatars: HashMap<String, Avatar>,
    #[serde(skip)]
    pub state: RoomState,
    pub exits: Vec<ExitData>,
    pub acl: RoomAcl,
    pub descriptions: HashMap<String, String>,
    pub url: String, // Full DID URL (did:ma:<ipns>#<room-id>)
}

#[derive(Clone, Debug)]
pub struct RoomState {
    pub avatars: TtlCache<String, String>,
    pub dispatch_queue: VecDeque<RoomDispatchTask>,
    pub events: VecDeque<RoomEvent>,
    pub next_event_sequence: u64,
}

#[derive(Clone, Debug)]
pub enum RoomDispatchTask {
    PresenceSnapshot,
    PresenceRoomStateTo(String),
    PresenceRefreshRequest,
    RoomEventsSince(u64),
    WorldBroadcast(String),
}

impl Default for RoomState {
    fn default() -> Self {
        Self::new(Duration::from_secs(DEFAULT_AVATAR_STATE_TTL_SECS))
    }
}

impl RoomState {
    pub fn new(ttl: Duration) -> Self {
        Self {
            avatars: TtlCache::with_capacity(ttl, 1024),
            dispatch_queue: VecDeque::new(),
            events: VecDeque::new(),
            next_event_sequence: 0,
        }
    }

    pub fn set_avatar_ttl(&mut self, ttl: Duration) {
        self.avatars.set_default_max_cache(ttl);
    }

    pub fn touch_avatar(&mut self, url: &str, handle: &str) {
        self.avatars.insert(url.to_string(), handle.to_string());
    }

    pub fn remove_avatar(&mut self, url: &str) {
        self.avatars.remove(&url.to_string());
    }

    pub fn enqueue_dispatch(&mut self, task: RoomDispatchTask) {
        self.dispatch_queue.push_back(task);
    }

    pub fn drain_dispatch_queue(&mut self) -> Vec<RoomDispatchTask> {
        self.dispatch_queue.drain(..).collect()
    }

    pub fn push_event(&mut self, max_events: usize, entry: RoomEvent) {
        if self.events.len() >= max_events {
            self.events.pop_front();
        }
        self.events.push_back(entry);
    }
}

fn default_room_title(name: &str) -> String {
    let mut words = Vec::new();
    for part in name.split(['-', '_', ' ']).filter(|p| !p.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            let mut word = first.to_uppercase().collect::<String>();
            word.push_str(chars.as_str());
            words.push(word);
        }
    }
    if words.is_empty() {
        "Room".to_string()
    } else {
        words.join(" ")
    }
}

impl Room {
    pub fn new(name: String, url: String) -> Self {
        Self {
            name,
            titles: HashMap::new(),
            avatars: HashMap::new(),
            state: RoomState::default(),
            exits: Vec::new(),
            acl: RoomAcl::open(),
            descriptions: HashMap::new(),
            url,
        }
    }

    pub fn set_title(&mut self, value: String) {
        self.titles.insert("und".to_string(), value);
    }

    pub fn set_description(&mut self, value: String) {
        self.descriptions.insert("und".to_string(), value);
    }

    pub fn description_or_default(&self) -> String {
        self.descriptions
            .get("und")
            .or_else(|| self.descriptions.get("und"))
            .cloned()
            .unwrap_or_else(|| "(no description)".to_string())
    }

    pub fn title_or_default(&self) -> String {
        self.titles
            .get("und")
            .or_else(|| self.titles.get("und"))
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_room_title(&self.name))
    }

    pub fn add_avatar(&mut self, avatar: Avatar) {
        self.state.touch_avatar(&avatar.agent_did.id(), &avatar.inbox);
        self.avatars.insert(avatar.inbox.clone(), avatar);
    }

    pub fn remove_avatar(&mut self, name: &str) {
        if let Some(avatar) = self.avatars.remove(name) {
            self.state.remove_avatar(&avatar.agent_did.id());
        }
    }
}

pub fn parse_room_inbox_symbol(symbol: &str) -> Option<&str> {
    let trimmed = symbol.trim();
    let rest = trimmed.strip_prefix("room.")?;
    let object = rest.strip_suffix(".inbox")?;
    let object = object.trim();
    if object.is_empty() {
        None
    } else {
        Some(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_room_inbox_symbol() {
        assert_eq!(parse_room_inbox_symbol("room.lobby.inbox"), Some("lobby"));
        assert_eq!(parse_room_inbox_symbol("room.abc-123.inbox"), Some("abc-123"));
    }

    #[test]
    fn rejects_invalid_inbox_symbols() {
        assert_eq!(parse_room_inbox_symbol("room..inbox"), None);
        assert_eq!(parse_room_inbox_symbol("notaroom.lobby.inbox"), None);
        assert_eq!(parse_room_inbox_symbol("room.lobby"), None);
    }
}
