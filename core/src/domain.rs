use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorType {
    World,
    Room,
    Avatar,
    Exit,
    Object,
}

/// A directed exit from a room to another room.
///
/// The source room is implied by which `RoomActor` contains this exit.
/// The `id` is a world-scoped DID fragment (e.g. a nanoid like `"abc123"` giving
/// `did:ma:<world-ipns>#abc123`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExitAcl {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl Default for ExitAcl {
    fn default() -> Self {
        Self::open()
    }
}

impl ExitAcl {
    pub fn open() -> Self {
        Self {
            allow: vec!["*".to_string()],
            deny: Vec::new(),
        }
    }

    pub fn can_use(&self, identity: &str) -> bool {
        if self.deny.iter().any(|entry| entry == identity) {
            return false;
        }
        self.allow
            .iter()
            .any(|entry| entry == "*" || entry == identity)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExitData {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub names: HashMap<String, String>,
    pub to: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub one_way: bool,
    #[serde(default)]
    pub acl: ExitAcl,
    #[serde(default)]
    pub travel_texts: HashMap<String, String>,
}

impl ExitData {
    pub fn new(id: impl Into<String>, name: impl Into<String>, to: impl Into<String>) -> Self {
        let canonical_name = name.into();
        let mut names = HashMap::new();
        if !canonical_name.trim().is_empty() {
            names.insert("und".to_string(), canonical_name.clone());
        }

        Self {
            id: id.into(),
            name: canonical_name,
            names,
            to: to.into(),
            aliases: Vec::new(),
            hidden: false,
            locked: false,
            one_way: false,
            acl: ExitAcl::open(),
            travel_texts: HashMap::new(),
        }
    }

    /// Returns true if `input` matches this exit's canonical name or any alias.
    pub fn matches(&self, input: &str) -> bool {
        let input = input.trim();
        self.name == input
            || self.aliases.iter().any(|a| a == input)
            || self.names.values().any(|n| n == input)
    }

    pub fn name_for_preferences(&self, preferences: &[String]) -> String {
        for candidate in preferences {
            let normalized = candidate.trim().replace('-', "_");
            if normalized.is_empty() {
                continue;
            }
            if let Some(value) = self.names.get(&normalized) {
                return value.clone();
            }
            if let Some((primary, _)) = normalized.split_once('-') {
                if let Some(value) = self.names.get(primary) {
                    return value.clone();
                }
            }
            if let Some((primary, _)) = normalized.split_once('_') {
                if let Some(value) = self.names.get(primary) {
                    return value.clone();
                }
            }
        }

        if let Some(value) = self.names.get("und") {
            return value.clone();
        }

        if let Some(value) = self.names.values().next() {
            return value.clone();
        }

        self.name.clone()
    }

    pub fn matches_for_preferences(&self, input: &str, preferences: &[String]) -> bool {
        let token = input.trim();
        if token.is_empty() {
            return false;
        }
        if self.matches(token) {
            return true;
        }
        if self.name_for_preferences(preferences) == token {
            return true;
        }

        let matches_direction_stem = |value: &str| {
            let trimmed = value.trim();
            if trimmed == token {
                return true;
            }
            if let Some((head, _)) = trimmed
                .split_once(" to ")
                .or_else(|| trimmed.split_once(" til "))
            {
                return head.trim() == token;
            }
            false
        };

        if matches_direction_stem(&self.name) {
            return true;
        }
        if self
            .names
            .values()
            .any(|value| matches_direction_stem(value))
        {
            return true;
        }
        self.aliases
            .iter()
            .any(|value| matches_direction_stem(value))
    }

    pub fn can_use(&self, identity: &str) -> bool {
        self.acl.can_use(identity)
    }

    pub fn travel_text_for_preferences(&self, preferences: &[String]) -> Option<String> {
        for candidate in preferences {
            let normalized = candidate.trim().replace('-', "_");
            if normalized.is_empty() {
                continue;
            }
            if let Some(value) = self.travel_texts.get(&normalized) {
                return Some(value.clone());
            }
            if let Some((primary, _)) = normalized.split_once('_') {
                if let Some(value) = self.travel_texts.get(primary) {
                    return Some(value.clone());
                }
            }
        }
        self.travel_texts
            .get("und")
            .cloned()
            .or_else(|| self.travel_texts.values().next().cloned())
    }
}

/// A generic room-local object (item, prop, container, etc.).
///
/// Like exits, objects are owned by the room's content graph, not independently published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldActor {
    pub id: String,
    pub display_name: String,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomActor {
    pub id: String,
    pub display_name: String,
    pub exits: Vec<ExitData>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarActor {
    pub id: String,
    pub display_name: String,
    pub commands: Vec<String>,
}

impl WorldActor {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            commands: vec![
                "help".to_string(),
                "who".to_string(),
                "l".to_string(),
                "acl".to_string(),
            ],
        }
    }
}

impl RoomActor {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            exits: Vec::new(),
            commands: vec![
                "help".to_string(),
                "who".to_string(),
                "l".to_string(),
                "acl".to_string(),
                "invite <did>".to_string(),
                "deny <did>".to_string(),
                "kick <handle>".to_string(),
                "go <exit>".to_string(),
            ],
        }
    }
}

impl AvatarActor {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            commands: vec!["describe \"...\"".to_string(), "show".to_string()],
        }
    }
}
