use crate::parse_property_command_for_keys;
use crate::reply::{Reply, Scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomActorAction {
    None,
    Invite {
        identity: String,
    },
    Deny {
        identity: String,
    },
    Kick {
        handle: String,
    },
    /// Create (or link) a room and connect it via a named exit.
    /// `destination` is the room name/fragment (without `#`).
    /// If `None`, the world auto-names the destination.
    Dig {
        exit_name: String,
        destination: Option<String>,
    },
    /// Remove a named exit from the current room.
    Bury {
        exit_name: String,
    },
    /// Set room attribute using a key/value pair.
    /// Supported keys: owner, title, description, cid, content-b64, exit-content-b64.
    SetAttribute {
        key: String,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomActorResult {
    pub response: String,
    pub action: RoomActorAction,
}

#[derive(Debug, Clone)]
pub struct RoomActorContext<'a> {
    pub room_name: &'a str,
    pub room_exists: bool,
    /// (handle, identity DID) pairs for avatars present in the room.
    pub avatars: Vec<(String, String)>,
    pub things: Vec<String>,
    pub acl_owner: Option<&'a str>,
    pub acl_summary: &'a str,
    /// Full DID URL of the caller (`did:ma:<ipns>#<fragment>`).
    pub caller_url: Option<&'a str>,
    /// Root DID (identity) of the caller.
    pub caller_owner: Option<&'a str>,
    pub description: &'a str,
    pub title: &'a str,
    /// Full DID URL of this room (`did:ma:<ipns>#<room-id>`).
    pub url: Option<&'a str>,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn none(reply: Reply) -> RoomActorResult {
    RoomActorResult {
        response: reply.to_string(),
        action: RoomActorAction::None,
    }
}

fn with_action(reply: Reply, action: RoomActorAction) -> RoomActorResult {
    RoomActorResult {
        response: reply.to_string(),
        action,
    }
}

fn room_not_found(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    none(Reply::here(format!("room '{}' not found", ctx.room_name)))
}

fn require_room(ctx: &RoomActorContext<'_>) -> Option<RoomActorResult> {
    if ctx.room_exists {
        None
    } else {
        Some(room_not_found(ctx))
    }
}

fn require_owner(ctx: &RoomActorContext<'_>, action_name: &str) -> Option<RoomActorResult> {
    if let Some(err) = require_room(ctx) {
        return Some(err);
    }
    if is_owner(ctx) {
        return None;
    }
    Some(none(Reply::here(format!(
        "only the room owner can {} in '{}'",
        action_name, ctx.room_name
    ))))
}

fn is_owner(ctx: &RoomActorContext<'_>) -> bool {
    let Some(owner) = ctx.acl_owner else {
        return false;
    };
    ctx.caller_url
        .map(|caller| caller == owner)
        .unwrap_or(false)
        || ctx
            .caller_owner
            .map(|caller_owner| caller_owner == owner)
            .unwrap_or(false)
}

fn is_owner_or_unclaimed(ctx: &RoomActorContext<'_>) -> bool {
    match (ctx.acl_owner, ctx.caller_url, ctx.caller_owner) {
        (None, _, _) => true,
        (Some(owner), Some(caller), caller_owner) => {
            owner == caller || caller_owner.map(|value| value == owner).unwrap_or(false)
        }
        _ => false,
    }
}

fn unquote(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

fn split_method_arg(input: &str) -> (String, String) {
    let mut parts = input.splitn(2, char::is_whitespace);
    let method = parts.next().unwrap_or_default().to_ascii_lowercase();
    let arg = parts.next().unwrap_or_default().trim().to_string();
    (method, arg)
}

fn format_avatar_list(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        "(none)".to_string()
    } else {
        pairs
            .iter()
            .map(|(handle, identity)| format!("{}({})", handle, identity))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ─── Built-in commands ──────────────────────────────────────────────────────

const BUILTIN_COMMANDS: &[&str] = &[
    "help",
    "who",
    "l",
    "acl",
    "describe",
    "show",
    "invite <did>",
    "deny <did>",
    "kick <handle>",
    "dig <direction> [to|til <#dest|did:ma:...#room>]",
    "bury <direction>",
    "owner [did]",
    "title [value]",
    "description [value]",
    "cid [value]",
    "content-b64 [value]",
    "exit-content-b64 [value]",
];

fn cmd_help(_ctx: &RoomActorContext<'_>) -> RoomActorResult {
    none(Reply::here(format!(
        "commands: {}",
        BUILTIN_COMMANDS.join(" | ")
    )))
}

fn cmd_who(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_room(ctx) {
        return err;
    }
    let mut pairs = ctx.avatars.clone();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    none(Reply::here_attr(
        "avatars",
        format!("in '{}': {}", ctx.room_name, format_avatar_list(&pairs)),
    ))
}

fn cmd_list(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_room(ctx) {
        return err;
    }
    let mut pairs = ctx.avatars.clone();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let avatars = format_avatar_list(&pairs);
    let mut things = ctx.things.clone();
    things.sort();
    let things = if things.is_empty() {
        "(none)".to_string()
    } else {
        things.join(", ")
    };
    none(Reply::here(format!(
        "room='{}' .avatars=[{}] .things=[{}]",
        ctx.room_name, avatars, things
    )))
}

fn cmd_acl(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_room(ctx) {
        return err;
    }
    let owner = ctx.acl_owner.unwrap_or("(none)");
    none(Reply::here_attr(
        "acl",
        format!(
            "for '{}': owner={} policy={}",
            ctx.room_name, owner, ctx.acl_summary
        ),
    ))
}

fn cmd_describe(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_room(ctx) {
        return err;
    }
    let desc = if ctx.description.is_empty() {
        "(no description)"
    } else {
        ctx.description
    };
    none(Reply::here(format!("{} — {}", ctx.room_name, desc)))
}

fn cmd_show(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_room(ctx) {
        return err;
    }
    let owner = ctx.acl_owner.unwrap_or("(none)");
    let url = ctx.url.unwrap_or("(unknown)");
    none(Reply::here(format!(
        "'{}': url={} owner={}",
        ctx.room_name, url, owner
    )))
}

fn cmd_invite_deny_kick(method: &str, arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_owner(ctx, &format!("run /{}", method)) {
        return err;
    }
    if arg.is_empty() {
        let usage = if method == "kick" {
            "kick <handle>"
        } else {
            "invite <did> | deny <did>"
        };
        return none(Reply::here(format!("usage: @here {}", usage)));
    }
    match method {
        "invite" => with_action(
            Reply::here(format!("{} invited to '{}'", arg, ctx.room_name)),
            RoomActorAction::Invite {
                identity: arg.to_string(),
            },
        ),
        "deny" => with_action(
            Reply::here(format!("{} denied from '{}'", arg, ctx.room_name)),
            RoomActorAction::Deny {
                identity: arg.to_string(),
            },
        ),
        "kick" => with_action(
            Reply::here(format!("{} was kicked from '{}'", arg, ctx.room_name)),
            RoomActorAction::Kick {
                handle: arg.to_string(),
            },
        ),
        _ => none(Reply::here(format!("unknown command: {}", method))),
    }
}

fn cmd_dig(arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_owner(ctx, "dig exits") {
        return err;
    }
    if arg.is_empty() {
        return none(Reply::here(
            "usage: @here dig <direction> [to|til <#dest|did:ma:...#room>]".to_string(),
        ));
    }
    let (exit_name, destination) =
        if let Some((dir, dest)) = arg.split_once(" to ").or_else(|| arg.split_once(" til ")) {
            let dest_clean = dest.trim().trim_start_matches('#').to_string();
            (
                dir.trim().to_string(),
                if dest_clean.is_empty() {
                    None
                } else {
                    Some(dest_clean)
                },
            )
        } else {
            (arg.to_string(), None)
        };
    let dest_display = destination.as_deref().unwrap_or("(auto)");
    with_action(
        Reply::here(format!(
            "exit '{}' dug from '{}' → {}",
            exit_name, ctx.room_name, dest_display
        )),
        RoomActorAction::Dig {
            exit_name,
            destination,
        },
    )
}

fn cmd_bury(arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_owner(ctx, "bury exits") {
        return err;
    }
    let exit_name = arg.trim();
    if exit_name.is_empty() {
        return none(Reply::here("usage: @here bury <direction>".to_string()));
    }
    with_action(
        Reply::here(format!(
            "exit '{}' buried in '{}'",
            exit_name, ctx.room_name
        )),
        RoomActorAction::Bury {
            exit_name: exit_name.to_string(),
        },
    )
}

fn cmd_set(arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if let Some(err) = require_room(ctx) {
        return err;
    }

    let mut kv = arg.splitn(2, char::is_whitespace);
    let key = kv.next().unwrap_or_default().trim().to_ascii_lowercase();
    let mut value_raw = kv.next().unwrap_or_default().trim().to_string();

    if let Some(rest) = value_raw.strip_prefix("to ") {
        value_raw = rest.trim().to_string();
    }

    let value = unquote(&value_raw);

    if key.is_empty() || value.is_empty() {
        return none(Reply::here(
            "usage: @here.<owner|title|description|cid|content-b64|exit-content-b64> [value]"
                .to_string(),
        ));
    }

    match key.as_str() {
        "owner" => {
            if !is_owner_or_unclaimed(ctx) {
                return none(Reply::here(format!("only the room owner can change ownership of '{}'", ctx.room_name)));
            }
            with_action(
                Reply::here(format!("'{}' is now owned by {}", ctx.room_name, value)),
                RoomActorAction::SetAttribute { key, value },
            )
        }
        "title" | "description" => {
            if !is_owner(ctx) {
                return none(Reply::here(format!("only the room owner can change {} of '{}'", key, ctx.room_name)));
            }
            with_action(
                Reply::here(format!("{} for '{}' updated", key, ctx.room_name)),
                RoomActorAction::SetAttribute { key, value },
            )
        }
        "cid" => {
            if !is_owner_or_unclaimed(ctx) {
                return none(Reply::here(format!("only the room owner can replace the CID of '{}'", ctx.room_name)));
            }
            with_action(
                Reply::here(format!("loading room '{}' from {}", ctx.room_name, value)),
                RoomActorAction::SetAttribute { key, value },
            )
        }
        "content" | "content-b64" => {
            if !is_owner_or_unclaimed(ctx) {
                return none(Reply::here(format!("only the room owner can replace room content for '{}'", ctx.room_name)));
            }
            with_action(
                Reply::here(format!("publishing room '{}' from inline content", ctx.room_name)),
                RoomActorAction::SetAttribute { key, value },
            )
        }
        "exit-content-b64" => {
            if !is_owner_or_unclaimed(ctx) {
                return none(Reply::here(format!("only the room owner can replace exit content for '{}'", ctx.room_name)));
            }
            with_action(
                Reply::here(format!("publishing exit content for '{}'", ctx.room_name)),
                RoomActorAction::SetAttribute { key, value },
            )
        }
        _ => none(Reply::here(format!("unknown set attribute '{}'. Supported: owner, title, description, cid, content-b64, exit-content-b64", key))),
    }
}

const ROOM_PROPERTY_KEYS: &[&str] = &[
    "_list",
    "owner",
    "title",
    "description",
    "cid",
    "content-b64",
    "exit-content-b64",
];

fn cmd_property(command: &str, ctx: &RoomActorContext<'_>) -> Option<RoomActorResult> {
    let property = parse_property_command_for_keys(command, ROOM_PROPERTY_KEYS)?;
    let key = property.key;

    if key == "_list" {
        let owner = ctx.acl_owner.unwrap_or("(none)");
        let url = ctx.url.unwrap_or("(unknown)");
        let response = Reply::attr_list(
            Scope::Here,
            &[
                ("owner", owner),
                ("title", ctx.title),
                ("description", ctx.description),
                ("url", url),
            ],
        );
        return Some(RoomActorResult {
            response,
            action: RoomActorAction::None,
        });
    }

    let Some(value) = property.value else {
        let response = match key.as_str() {
            "owner" => ctx.acl_owner.unwrap_or("(none)").to_string(),
            "title" => ctx.title.to_string(),
            "description" => ctx.description.to_string(),
            "cid" | "content-b64" | "exit-content-b64" => "(write-only)".to_string(),
            _ => Reply::here(format!(
                "unknown attribute '{}'. Supported: owner, title, description, cid, content-b64, exit-content-b64",
                key
            )).to_string(),
        };
        return Some(none(Reply::here(response)));
    };

    Some(cmd_set(&format!("{} {}", key, value), ctx))
}

// ─── Main dispatch ──────────────────────────────────────────────────────────

pub fn execute_room_actor_command(command: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    let normalized = command.trim();

    if normalized.is_empty() {
        return cmd_show(ctx);
    }

    if normalized.eq_ignore_ascii_case("help") {
        return cmd_help(ctx);
    }

    if normalized.to_ascii_lowercase().starts_with("set ") {
        return none(Reply::here(
            "'set ...' is deprecated. Use dot notation: @here.<owner|title|description|cid|content-b64|exit-content-b64> [value]".to_string()
        ));
    }

    if let Some(result) = cmd_property(normalized, ctx) {
        return result;
    }

    let (method, arg) = split_method_arg(normalized);

    // Query commands (read-only, no permissions beyond room existence).
    match method.as_str() {
        "who" | "actors" => return cmd_who(ctx),
        "l" | "list" => return cmd_list(ctx),
        "acl" => return cmd_acl(ctx),
        "describe" => return cmd_describe(ctx),
        "show" => return cmd_show(ctx),
        _ => {}
    }

    // Mutation commands (require owner).
    match method.as_str() {
        "invite" | "deny" | "kick" => return cmd_invite_deny_kick(&method, &arg, ctx),
        "dig" => return cmd_dig(&arg, ctx),
        "bury" => return cmd_bury(&arg, ctx),
        _ => {}
    }

    cmd_help(ctx)
}

#[cfg(test)]
mod tests {
    use super::{execute_room_actor_command, RoomActorAction, RoomActorContext};

    fn sample_ctx() -> RoomActorContext<'static> {
        RoomActorContext {
            room_name: "lobby",
            room_exists: true,
            avatars: vec![("aurora".to_string(), "did:ma:k51actor123".to_string())],
            things: Vec::new(),
            acl_owner: Some("did:ma:owner"),
            acl_summary: "*",
            caller_url: Some("did:ma:owner"),
            caller_owner: Some("did:ma:owner-root"),
            title: "Lobby",
            description: "Welcome",
            url: Some("did:ma:world#lobby"),
        }
    }

    #[test]
    fn supports_owner_dot_notation() {
        let result = execute_room_actor_command("owner did:ma:new-owner", &sample_ctx());
        assert!(matches!(
            result.action,
            RoomActorAction::SetAttribute { ref key, ref value }
            if key == "owner" && value == "did:ma:new-owner"
        ));
    }

    #[test]
    fn rejects_legacy_set_syntax() {
        let result = execute_room_actor_command("set owner did:ma:new-owner", &sample_ctx());
        assert!(result.response.contains("deprecated"));
        assert!(matches!(result.action, RoomActorAction::None));
    }

    #[test]
    fn supports_bury_command() {
        let result = execute_room_actor_command("bury north", &sample_ctx());
        assert!(matches!(
            result.action,
            RoomActorAction::Bury { ref exit_name } if exit_name == "north"
        ));
    }

    #[test]
    fn empty_command_defaults_to_show() {
        let result = execute_room_actor_command("", &sample_ctx());
        assert!(result
            .response
            .contains("'lobby': did=did:ma:world#lobby owner=did:ma:owner"));
        assert!(matches!(result.action, RoomActorAction::None));
    }

    #[test]
    fn unknown_command_falls_back_to_help() {
        let result = execute_room_actor_command("foobardoesntexist", &sample_ctx());
        assert!(result.response.starts_with("@here commands:"));
        assert!(result.response.contains("show"));
        assert!(matches!(result.action, RoomActorAction::None));
    }

    #[test]
    fn explicit_help_still_returns_help() {
        let result = execute_room_actor_command("help", &sample_ctx());
        assert!(result.response.starts_with("@here commands:"));
        assert!(result.response.contains("owner [did]"));
        assert!(matches!(result.action, RoomActorAction::None));
    }

    #[test]
    fn who_uses_dot_notation() {
        let result = execute_room_actor_command("who", &sample_ctx());
        assert!(result.response.starts_with("@here.avatars"));
        assert!(result.response.contains("aurora(did:ma:k51actor123)"));
    }

    #[test]
    fn acl_uses_dot_notation() {
        let result = execute_room_actor_command("acl", &sample_ctx());
        assert!(result.response.starts_with("@here.acl"));
    }
}
