/// Structured response formatting for world commands.
///
/// All command responses go through `Reply` so that the `@scope` prefix
/// notation is defined in exactly one place.

/// The scope prefix for a command response (e.g. `@here`, `@world`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Here,
    World,
    Avatar,
    /// A named scope, e.g. a DID or custom target.
    Named(String),
}

impl Scope {
    fn prefix(&self) -> &str {
        match self {
            Self::Here => "@here",
            Self::World => "@world",
            Self::Avatar => "@avatar",
            Self::Named(name) => name,
        }
    }
}

/// A structured response from a command handler.
///
/// Use `Reply::here(msg)` / `Reply::world(msg)` for simple responses,
/// or `Reply::here_attr("avatars", msg)` for dot-notated attribute responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub scope: Scope,
    pub attribute: Option<String>,
    pub message: String,
}

impl Reply {
    pub fn new(scope: Scope, message: impl Into<String>) -> Self {
        Self { scope, attribute: None, message: message.into() }
    }

    pub fn with_attr(scope: Scope, attr: impl Into<String>, message: impl Into<String>) -> Self {
        Self { scope, attribute: Some(attr.into()), message: message.into() }
    }

    pub fn here(message: impl Into<String>) -> Self {
        Self::new(Scope::Here, message)
    }

    pub fn here_attr(attr: impl Into<String>, message: impl Into<String>) -> Self {
        Self::with_attr(Scope::Here, attr, message)
    }

    pub fn world(message: impl Into<String>) -> Self {
        Self::new(Scope::World, message)
    }

    pub fn world_attr(attr: impl Into<String>, message: impl Into<String>) -> Self {
        Self::with_attr(Scope::World, attr, message)
    }

    /// Join multiple replies into a single newline-separated string.
    pub fn join(replies: &[Reply]) -> String {
        replies.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n")
    }

    /// Build a list of `@scope.attr value` lines from key-value pairs.
    pub fn attr_list(scope: Scope, pairs: &[(&str, &str)]) -> String {
        let replies: Vec<Reply> = pairs
            .iter()
            .map(|(k, v)| Reply::with_attr(scope.clone(), *k, *v))
            .collect();
        Self::join(&replies)
    }
}

impl std::fmt::Display for Reply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.attribute {
            Some(attr) => write!(f, "{}.{} {}", self.scope.prefix(), attr, self.message),
            None => write!(f, "{} {}", self.scope.prefix(), self.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn here_simple() {
        let r = Reply::here("room 'lobby' not found");
        assert_eq!(r.to_string(), "@here room 'lobby' not found");
    }

    #[test]
    fn here_with_attribute() {
        let r = Reply::here_attr("avatars", "in 'lobby': aurora(did:ma:k51abc)");
        assert_eq!(r.to_string(), "@here.avatars in 'lobby': aurora(did:ma:k51abc)");
    }

    #[test]
    fn world_simple() {
        let r = Reply::world("claimed by did:ma:k51xyz");
        assert_eq!(r.to_string(), "@world claimed by did:ma:k51xyz");
    }

    #[test]
    fn world_with_attribute() {
        let r = Reply::world_attr("avatars", "(none)");
        assert_eq!(r.to_string(), "@world.avatars (none)");
    }

    #[test]
    fn named_scope() {
        let r = Reply::new(Scope::Named("@did:ma:k51abc".into()), "pong");
        assert_eq!(r.to_string(), "@did:ma:k51abc pong");
    }

    #[test]
    fn join_lines() {
        let text = Reply::join(&[
            Reply::here_attr("title", "Lobby"),
            Reply::here_attr("did", "did:ma:k51abc#lobby"),
        ]);
        assert_eq!(text, "@here.title Lobby\n@here.did did:ma:k51abc#lobby");
    }

    #[test]
    fn attr_list_builds_prefixed_lines() {
        let text = Reply::attr_list(Scope::World, &[("owner", "alice"), ("did", "did:ma:k51x")]);
        assert_eq!(text, "@world.owner alice\n@world.did did:ma:k51x");
    }
}
