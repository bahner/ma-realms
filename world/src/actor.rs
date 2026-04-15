use did_ma::Did;
use std::collections::HashMap;

/// 32-byte Ed25519 private key for avatar signing, held by the world.
pub type AvatarSigningSecret = [u8; 32];

/// Request to join a world room, carrying all the agent-provided identity fields.
#[derive(Clone, Debug)]
pub(crate) struct AvatarRequest {
    pub did: Did,
    pub owner: String,
    pub agent_endpoint: String,
    pub language_order: String,
    pub signing_secret: AvatarSigningSecret,
    pub encryption_pubkey_multibase: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ActorAcl {
    pub allow_all: bool,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl ActorAcl {
    pub fn open() -> Self {
        Self {
            allow_all: true,
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.allow_all {
            parts.push("*".to_string());
        }
        for did in &self.allow {
            parts.push(did.clone());
        }
        for did in &self.deny {
            parts.push(format!("!{did}"));
        }
        if parts.is_empty() {
            "(none)".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// World-local representation of a connected agent.
/// Knows the agent's root DID and iroh endpoint so the world
/// can route ma-messages back to them.
#[derive(Clone, Debug)]
pub struct Avatar {
    pub inbox: String,
    pub agent_did: Did,
    pub agent_endpoint: String,
    pub language_order: String,
    pub owner: String,
    pub descriptions: HashMap<String, String>,
    pub object_shortcuts: HashMap<String, String>,
    pub acl: ActorAcl,
    pub joined_at: std::time::SystemTime,
    pub last_seen_at: std::time::SystemTime,
    /// Ed25519 private key bytes — world signs on behalf of the avatar.
    pub _signing_secret: AvatarSigningSecret,
    /// Actor's X25519 encryption public key (multibase), for keyAgreement.
    pub encryption_pubkey_multibase: Option<String>,
}

impl Avatar {
    pub fn new(
        inbox: String,
        agent_did: Did,
        agent_endpoint: String,
        language_order: String,
        owner: String,
        signing_secret: AvatarSigningSecret,
        encryption_pubkey_multibase: Option<String>,
    ) -> Self {
        Self {
            inbox,
            agent_did,
            agent_endpoint,
            language_order,
            owner,
            descriptions: HashMap::new(),
            object_shortcuts: HashMap::new(),
            acl: ActorAcl::open(),
            joined_at: std::time::SystemTime::now(),
            last_seen_at: std::time::SystemTime::now(),
            _signing_secret: signing_secret,
            encryption_pubkey_multibase,
        }
    }

    pub fn touch_presence(&mut self) {
        self.last_seen_at = std::time::SystemTime::now();
    }

    fn normalize_shortcut(alias: &str) -> Option<String> {
        let trimmed = alias.trim().trim_start_matches('@').to_ascii_lowercase();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            Some(trimmed)
        } else {
            None
        }
    }

    pub fn bind_object_shortcut(&mut self, alias: &str, object_id: &str) -> bool {
        let Some(normalized) = Self::normalize_shortcut(alias) else {
            return false;
        };
        self.object_shortcuts
            .insert(normalized, object_id.trim().to_string());
        true
    }

    pub fn resolve_object_shortcut(&self, alias: &str) -> Option<String> {
        let normalized = Self::normalize_shortcut(alias)?;
        self.object_shortcuts.get(&normalized).cloned()
    }

    pub fn remove_object_shortcut(&mut self, alias: &str) -> bool {
        let Some(normalized) = Self::normalize_shortcut(alias) else {
            return false;
        };
        self.object_shortcuts.remove(&normalized).is_some()
    }

    pub fn object_shortcuts_summary(&self) -> String {
        if self.object_shortcuts.is_empty() {
            return "(none)".to_string();
        }
        let mut rows = self
            .object_shortcuts
            .iter()
            .map(|(alias, object_id)| format!("@{}->{}", alias, object_id))
            .collect::<Vec<_>>();
        rows.sort();
        rows.join(", ")
    }

    pub fn set_description(&mut self, value: String) {
        self.descriptions.insert("und".to_string(), value);
    }

    pub fn description_or_default(&self) -> String {
        self.descriptions
            .get("und")
            .or_else(|| self.descriptions.get("und"))
            .cloned()
            .unwrap_or_else(|| "(unset)".to_string())
    }
}
