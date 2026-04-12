use crate::lang::{tr_world, world_lang_from_profile};
use ma_core::ExitData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AvatarAction {
    None,
    Move { exit: ExitData },
}

#[derive(Debug, Clone)]
pub(crate) struct AvatarCommandResult {
    pub response: String,
    pub action: AvatarAction,
}

pub(crate) struct AvatarCommandContext {
    pub room_name: String,
    pub room_title: String,
    pub room_description: String,
    pub exits: Vec<ExitData>,
    pub avatars: Vec<String>,
    pub things: Vec<String>,
    pub sender_profile: String,
    pub caller_did: String,
}

impl AvatarCommandContext {
    fn lang(&self) -> &'static str {
        world_lang_from_profile(&self.sender_profile)
    }

    fn tr(&self, key: &str, fallback: &str) -> String {
        tr_world(self.lang(), key, fallback)
    }

    fn lang_prefs(&self) -> Vec<String> {
        self.sender_profile
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

fn none_result(response: String) -> AvatarCommandResult {
    AvatarCommandResult {
        response,
        action: AvatarAction::None,
    }
}

pub(crate) fn execute_avatar_command(
    command: &str,
    ctx: &AvatarCommandContext,
) -> Option<AvatarCommandResult> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();

    if lower == "l" || lower == "look" {
        return Some(cmd_look(ctx));
    }

    if lower.starts_with("l ") || lower.starts_with("look ") {
        let target = if lower.starts_with("l ") {
            trimmed.get("l ".len()..).unwrap_or("").trim()
        } else {
            trimmed.get("look ".len()..).unwrap_or("").trim()
        };
        return Some(cmd_look_at(target, ctx));
    }

    if lower.starts_with("go ") || lower == "go" {
        let direction = trimmed
            .strip_prefix("go ")
            .or_else(|| trimmed.strip_prefix("Go "))
            .or_else(|| trimmed.strip_prefix("GO "))
            .map(str::trim)
            .unwrap_or("");
        return Some(cmd_go(direction, ctx));
    }

    if lower.starts_with("inspect ") || lower == "inspect" {
        let target = trimmed.get("inspect ".len()..).unwrap_or("").trim();
        return Some(cmd_inspect(target, ctx));
    }

    // Try bare direction matching (single word that matches an exit).
    if !trimmed.contains(' ') {
        if let Some(exit) = find_exit(trimmed, ctx) {
            return Some(cmd_go_exit(exit, ctx));
        }
    }

    None
}

// ─── look ───────────────────────────────────────────────────────────────────

fn cmd_look(ctx: &AvatarCommandContext) -> AvatarCommandResult {
    let prefs = ctx.lang_prefs();
    let mut lines: Vec<String> = Vec::new();

    // Title & description.
    let title = if ctx.room_title.is_empty() {
        ctx.room_name.to_string()
    } else {
        ctx.room_title.to_string()
    };
    let desc = if ctx.room_description.is_empty() || ctx.room_description == "(no description)" {
        ctx.tr("avatar.look.no-description", "Nothing special here.")
    } else {
        ctx.room_description.to_string()
    };
    lines.push(format!("── {} ──", title));
    lines.push(desc);

    // Exits.
    let visible_exits: Vec<&ExitData> = ctx.exits.iter().filter(|e| !e.hidden).collect();
    if visible_exits.is_empty() {
        lines.push(ctx.tr("avatar.look.no-exits", "There are no obvious exits."));
    } else {
        let exit_label = ctx.tr("avatar.look.exits-label", "Exits:");
        let exit_names: Vec<String> = visible_exits
            .iter()
            .map(|e| {
                let name = e.name_for_preferences(&prefs);
                if e.locked { format!("{} 🔒", name) } else { name }
            })
            .collect();
        lines.push(format!("{} {}", exit_label, exit_names.join(", ")));
    }

    // Avatars present (excluding the caller).
    let others: Vec<&String> = ctx
        .avatars
        .iter()
        .filter(|a| a.as_str() != ctx.caller_did)
        .collect();
    if !others.is_empty() {
        let present_label = ctx.tr("avatar.look.present-label", "Present:");
        let names: Vec<String> = others.iter().map(|a| a.to_string()).collect();
        lines.push(format!("{} {}", present_label, names.join(", ")));
    }

    // Things / objects.
    if !ctx.things.is_empty() {
        let things_label = ctx.tr("avatar.look.things-label", "You see:");
        lines.push(format!("{} {}", things_label, ctx.things.join(", ")));
    }

    none_result(lines.join("\n"))
}

// ─── look at ────────────────────────────────────────────────────────────────

fn cmd_look_at(target: &str, ctx: &AvatarCommandContext) -> AvatarCommandResult {
    if target.is_empty() {
        return cmd_look(ctx);
    }

    let prefs = ctx.lang_prefs();

    // Look at an exit?
    if let Some(exit) = ctx
        .exits
        .iter()
        .find(|e| e.matches_for_preferences(target, &prefs))
    {
        let name = exit.name_for_preferences(&prefs);
        let status = if exit.locked {
            ctx.tr("avatar.look.exit-locked", "It is locked.")
        } else {
            ctx.tr("avatar.look.exit-open", "It is open.")
        };
        return none_result(format!("{} — {}", name, status));
    }

    // Look at a thing/object?
    if ctx.things.iter().any(|t| t.eq_ignore_ascii_case(target)) {
        return none_result(target.to_string());
    }

    // Look at an avatar?
    if ctx.avatars.iter().any(|a| a.eq_ignore_ascii_case(target)) {
        return none_result(target.to_string());
    }

    none_result(format!(
        "{} '{}'",
        ctx.tr("avatar.look.nothing", "You see nothing called"),
        target
    ))
}

// ─── go ─────────────────────────────────────────────────────────────────────

fn find_exit(direction: &str, ctx: &AvatarCommandContext) -> Option<ExitData> {
    let prefs = ctx.lang_prefs();
    ctx.exits
        .iter()
        .find(|e| e.matches_for_preferences(direction, &prefs))
        .cloned()
}

fn cmd_go(direction: &str, ctx: &AvatarCommandContext) -> AvatarCommandResult {
    if direction.is_empty() {
        return none_result(ctx.tr("avatar.go.usage", "Go where?"));
    }
    match find_exit(direction, ctx) {
        Some(exit) => cmd_go_exit(exit, ctx),
        None => none_result(format!(
            "{} '{}'",
            ctx.tr("avatar.go.no-exit", "No exit"),
            direction
        )),
    }
}

fn cmd_go_exit(exit: ExitData, ctx: &AvatarCommandContext) -> AvatarCommandResult {
    let prefs = ctx.lang_prefs();
    let exit_name = exit.name_for_preferences(&prefs);

    if exit.locked {
        return none_result(format!(
            "{} {}",
            ctx.tr("avatar.go.locked", "The way is locked:"),
            exit_name
        ));
    }

    if !exit.can_use(&ctx.caller_did) {
        return none_result(format!(
            "{} '{}'",
            ctx.tr("avatar.go.denied", "You cannot use exit"),
            exit_name
        ));
    }

    AvatarCommandResult {
        response: String::new(),
        action: AvatarAction::Move { exit },
    }
}

// ─── inspect ────────────────────────────────────────────────────────────────

fn cmd_inspect(target: &str, ctx: &AvatarCommandContext) -> AvatarCommandResult {
    if target.is_empty() {
        return none_result(ctx.tr("avatar.inspect.usage", "Inspect what?"));
    }

    let prefs = ctx.lang_prefs();

    // Inspect an exit?
    if let Some(exit) = ctx
        .exits
        .iter()
        .find(|e| e.matches_for_preferences(target, &prefs))
    {
        let mut lines = vec![format!("@inspect exit id={}", exit.id)];
        lines.push(format!("  name={}", exit.name));
        if !exit.names.is_empty() {
            for (lang, name) in &exit.names {
                lines.push(format!("  name.{}={}", lang, name));
            }
        }
        lines.push(format!("  to={}", exit.to));
        if !exit.aliases.is_empty() {
            lines.push(format!("  aliases={}", exit.aliases.join(", ")));
        }
        lines.push(format!("  hidden={}", exit.hidden));
        lines.push(format!("  locked={}", exit.locked));
        lines.push(format!("  one_way={}", exit.one_way));
        if !exit.travel_texts.is_empty() {
            for (lang, text) in &exit.travel_texts {
                lines.push(format!("  travel_text.{}={}", lang, text));
            }
        }
        return none_result(lines.join("\n"));
    }

    // Inspect a thing/object?
    if ctx.things.iter().any(|t| t.eq_ignore_ascii_case(target)) {
        return none_result(format!(
            "@inspect object name={} room={}",
            target, ctx.room_name
        ));
    }

    // Inspect an avatar?
    if ctx.avatars.iter().any(|a| a.eq_ignore_ascii_case(target)) {
        return none_result(format!(
            "@inspect avatar handle={} room={}",
            target, ctx.room_name
        ));
    }

    none_result(format!(
        "{} '{}'",
        ctx.tr("avatar.inspect.not-found", "Nothing to inspect:"),
        target
    ))
}
