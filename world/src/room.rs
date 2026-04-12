use crate::actor::Avatar;
use ma_core::ExitData;
use std::collections::{HashMap, HashSet};

/// Access control list for a room.
/// Evaluation order: deny list → allow list (with * wildcard).
/// Owner access is enforced through ACL membership (allow/deny), not bypass rules.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RoomAcl {
    /// Root DID of the room owner.
    pub owner: Option<String>,
    /// Public assertion key (multibase) of current owner at time of assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_assertion_key: Option<String>,
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
            owner_assertion_key: None,
            allow,
            deny: HashSet::new(),
        }
    }

    /// Returns `true` if `did` is allowed to enter this room.
    pub fn can_enter(&self, did: &str) -> bool {
        // Explicit deny takes priority.
        if self.deny.contains(did) {
            return false;
        }
        // Wildcard allow or explicit allow.
        self.allow.contains("*") || self.allow.contains(did)
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
    pub exits: Vec<ExitData>,
    pub acl: RoomAcl,
    pub descriptions: HashMap<String, String>,
    pub did: String, // Full DID (with IPNS fragment)
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
    pub fn new(name: String, did: String) -> Self {
        Self {
            name,
            titles: HashMap::new(),
            avatars: HashMap::new(),
            exits: Vec::new(),
            acl: RoomAcl::open(),
            descriptions: HashMap::new(),
            did,
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
        self.avatars.insert(avatar.inbox.clone(), avatar);
    }

    pub fn remove_avatar(&mut self, name: &str) {
        self.avatars.remove(name);
    }
}
