use crate::parse_property_command_for_keys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomActorAction {
    None,
    Invite { did: String },
    Deny { did: String },
    Kick { handle: String },
    /// Create (or link) a room and connect it via a named exit.
    /// `destination` is the room name/fragment (without `#`).
    /// If `None`, the world auto-names the destination.
    Dig { exit_name: String, destination: Option<String> },
    /// Set room attribute using a key/value pair.
    /// Supported keys: owner, title, description, cid, content-b64, exit-content-b64.
    SetAttribute { key: String, value: String },
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
    pub avatars: Vec<String>,
    pub things: Vec<String>,
    pub acl_owner_did: Option<&'a str>,
    pub acl_summary: &'a str,
    pub caller_root_did: Option<&'a str>,
    pub description: &'a str,
    pub title: &'a str,
    pub did: Option<&'a str>,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn none_result(response: String) -> RoomActorResult {
    RoomActorResult { response, action: RoomActorAction::None }
}

fn room_not_found(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    none_result(format!("@here room '{}' not found", ctx.room_name))
}

fn is_owner(ctx: &RoomActorContext<'_>) -> bool {
    ctx.acl_owner_did
        .zip(ctx.caller_root_did)
        .map(|(owner, caller)| owner == caller)
        .unwrap_or(false)
}

fn is_owner_or_unclaimed(ctx: &RoomActorContext<'_>) -> bool {
    match (ctx.acl_owner_did, ctx.caller_root_did) {
        (None, _) => true,
        (Some(owner), Some(caller)) => owner == caller,
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

// ─── Built-in commands ──────────────────────────────────────────────────────

const BUILTIN_COMMANDS: &[&str] = &[
    "help", "who", "l", "acl", "describe", "show",
    "invite <did>", "deny <did>", "kick <handle>",
    "dig <direction> [to|til <#dest|did:ma:...#room>]",
    "owner [did]", "title [value]", "description [value]",
    "cid [value]", "content-b64 [value]", "exit-content-b64 [value]",
];

fn cmd_help(_ctx: &RoomActorContext<'_>) -> RoomActorResult {
    let commands = BUILTIN_COMMANDS.join(" | ");
    none_result(format!("@here commands: {}", commands))
}

fn cmd_who(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    let mut names = ctx.avatars.clone();
    names.sort();
    if names.is_empty() {
        none_result(format!("@here room '{}' has no actors", ctx.room_name))
    } else {
        none_result(format!("@here actors in '{}': {}", ctx.room_name, names.join(", ")))
    }
}

fn cmd_list(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    let mut names = ctx.avatars.clone();
    names.sort();
    let avatars = if names.is_empty() { "(none)".to_string() } else { names.join(", ") };
    let mut things = ctx.things.clone();
    things.sort();
    let things = if things.is_empty() { "(none)".to_string() } else { things.join(", ") };
    none_result(format!("@here room='{}' avatars=[{}] things=[{}]", ctx.room_name, avatars, things))
}

fn cmd_acl(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    let owner = ctx.acl_owner_did.unwrap_or("(none)");
    none_result(format!("@here acl for '{}': owner={} policy={}", ctx.room_name, owner, ctx.acl_summary))
}

fn cmd_describe(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    let desc = if ctx.description.is_empty() { "(no description)" } else { ctx.description };
    none_result(format!("@here {} — {}", ctx.room_name, desc))
}

fn cmd_show(ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    let owner = ctx.acl_owner_did.unwrap_or("(none)");
    let did = ctx.did.unwrap_or("(unknown)");
    none_result(format!("@here '{}': did={} owner={}", ctx.room_name, did, owner))
}

fn cmd_invite_deny_kick(method: &str, arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    if !is_owner(ctx) {
        return none_result(format!("@here only the room owner can run /{} in '{}'", method, ctx.room_name));
    }
    if arg.is_empty() {
        let usage = if method == "kick" { "kick <handle>" } else { "invite <did> | deny <did>" };
        return none_result(format!("@here usage: @here {}", usage));
    }
    match method {
        "invite" => RoomActorResult {
            response: format!("@here {} invited to '{}'", arg, ctx.room_name),
            action: RoomActorAction::Invite { did: arg.to_string() },
        },
        "deny" => RoomActorResult {
            response: format!("@here {} denied from '{}'", arg, ctx.room_name),
            action: RoomActorAction::Deny { did: arg.to_string() },
        },
        "kick" => RoomActorResult {
            response: format!("@here {} was kicked from '{}'", arg, ctx.room_name),
            action: RoomActorAction::Kick { handle: arg.to_string() },
        },
        _ => none_result(format!("@here unknown command: {}", method)),
    }
}

fn cmd_dig(arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }
    if !is_owner(ctx) {
        return none_result(format!("@here only the room owner can dig exits in '{}'", ctx.room_name));
    }
    if arg.is_empty() {
        return none_result("@here usage: @here dig <direction> [to|til <#dest|did:ma:...#room>]".to_string());
    }
    let (exit_name, destination) = if let Some((dir, dest)) = arg
        .split_once(" to ")
        .or_else(|| arg.split_once(" til "))
    {
        let dest_clean = dest.trim().trim_start_matches('#').to_string();
        (dir.trim().to_string(), if dest_clean.is_empty() { None } else { Some(dest_clean) })
    } else {
        (arg.to_string(), None)
    };
    let dest_display = destination.as_deref().unwrap_or("(auto)");
    RoomActorResult {
        response: format!("@here exit '{}' dug from '{}' → {}", exit_name, ctx.room_name, dest_display),
        action: RoomActorAction::Dig { exit_name, destination },
    }
}

fn cmd_set(arg: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    if !ctx.room_exists { return room_not_found(ctx); }

    let mut kv = arg.splitn(2, char::is_whitespace);
    let key = kv.next().unwrap_or_default().trim().to_ascii_lowercase();
    let mut value_raw = kv.next().unwrap_or_default().trim().to_string();

    if let Some(rest) = value_raw.strip_prefix("to ") {
        value_raw = rest.trim().to_string();
    }

    let value = unquote(&value_raw);

    if key.is_empty() || value.is_empty() {
        return none_result("@here usage: @here.<owner|title|description|cid|content-b64|exit-content-b64> [value]".to_string());
    }

    match key.as_str() {
        "owner" => {
            if !is_owner_or_unclaimed(ctx) {
                return none_result(format!("@here only the room owner can change ownership of '{}'", ctx.room_name));
            }
            RoomActorResult {
                response: format!("@here '{}' is now owned by {}", ctx.room_name, value),
                action: RoomActorAction::SetAttribute { key, value },
            }
        }
        "title" | "description" => {
            if !is_owner(ctx) {
                return none_result(format!("@here only the room owner can change {} of '{}'", key, ctx.room_name));
            }
            RoomActorResult {
                response: format!("@here {} for '{}' updated", key, ctx.room_name),
                action: RoomActorAction::SetAttribute { key, value },
            }
        }
        "cid" => {
            if !is_owner_or_unclaimed(ctx) {
                return none_result(format!("@here only the room owner can replace the CID of '{}'", ctx.room_name));
            }
            RoomActorResult {
                response: format!("@here loading room '{}' from {}", ctx.room_name, value),
                action: RoomActorAction::SetAttribute { key, value },
            }
        }
        "content" | "content-b64" => {
            if !is_owner_or_unclaimed(ctx) {
                return none_result(format!("@here only the room owner can replace room content for '{}'", ctx.room_name));
            }
            RoomActorResult {
                response: format!("@here publishing room '{}' from inline content", ctx.room_name),
                action: RoomActorAction::SetAttribute { key, value },
            }
        }
        "exit-content-b64" => {
            if !is_owner_or_unclaimed(ctx) {
                return none_result(format!("@here only the room owner can replace exit content for '{}'", ctx.room_name));
            }
            RoomActorResult {
                response: format!("@here publishing exit content for '{}'", ctx.room_name),
                action: RoomActorAction::SetAttribute { key, value },
            }
        }
        _ => none_result(format!("@here unknown set attribute '{}'. Supported: owner, title, description, cid, content-b64, exit-content-b64", key)),
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
        let owner = ctx.acl_owner_did.unwrap_or("(none)");
        let did = ctx.did.unwrap_or("(unknown)");
        return Some(none_result(format!(
            "@ .here.owner {}\n@ .here.title {}\n@ .here.description {}\n@ .here.did {}",
            owner,
            ctx.title,
            ctx.description,
            did
        )));
    }

    let Some(value) = property.value else {
        let response = match key.as_str() {
            "owner" => ctx.acl_owner_did.unwrap_or("(none)").to_string(),
            "title" => ctx.title.to_string(),
            "description" => ctx.description.to_string(),
            "cid" | "content-b64" | "exit-content-b64" => {
                "(write-only)".to_string()
            }
            _ => format!(
                "@here unknown attribute '{}'. Supported: owner, title, description, cid, content-b64, exit-content-b64",
                key
            ),
        };
        return Some(none_result(response));
    };

    let result = cmd_set(&format!("{} {}", key, value), ctx);
    Some(result)
}

// ─── Main dispatch ──────────────────────────────────────────────────────────

pub fn execute_room_actor_command(command: &str, ctx: &RoomActorContext<'_>) -> RoomActorResult {
    let normalized = command.trim();

    if normalized.is_empty() || normalized.eq_ignore_ascii_case("help") {
        return cmd_help(ctx);
    }

    if normalized.to_ascii_lowercase().starts_with("set ") {
        return none_result(
            "@here 'set ...' is deprecated. Use dot notation: @here.<owner|title|description|cid|content-b64|exit-content-b64> [value]".to_string()
        );
    }

    if let Some(result) = cmd_property(normalized, ctx) {
        return result;
    }

    let (method, arg) = split_method_arg(normalized);

    // Query commands (read-only, no permissions beyond room existence).
    match method.as_str() {
        "who" | "actors" => return cmd_who(ctx),
        "l" | "list"     => return cmd_list(ctx),
        "acl"            => return cmd_acl(ctx),
        "describe"       => return cmd_describe(ctx),
        "show"           => return cmd_show(ctx),
        _ => {}
    }

    // Mutation commands (require owner).
    match method.as_str() {
        "invite" | "deny" | "kick" => return cmd_invite_deny_kick(&method, &arg, ctx),
        "dig"                      => return cmd_dig(&arg, ctx),
        _ => {}
    }

    // TODO: Evaluator-registered commands go here.
    // When Lua/Guile evaluators are integrated, this is where user-defined
    // verbs would be dispatched, e.g.:
    //
    //   if let Some(result) = ctx.try_evaluator_command(&method, &arg) {
    //       return result;
    //   }

    none_result(format!("@here unknown command: {}", normalized))
}

#[cfg(test)]
mod tests {
    use super::{execute_room_actor_command, RoomActorAction, RoomActorContext};

    fn sample_ctx() -> RoomActorContext<'static> {
        RoomActorContext {
            room_name: "lobby",
            room_exists: true,
            avatars: vec!["aurora".to_string()],
            things: Vec::new(),
            acl_owner_did: Some("did:ma:owner"),
            acl_summary: "*",
            caller_root_did: Some("did:ma:owner"),
            title: "Lobby",
            description: "Welcome",
            did: Some("did:ma:world#lobby"),
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
}
