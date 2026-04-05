use serde::{Deserialize, Serialize};

fn canonical_target(target: &str) -> String {
    let normalized = target.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "here" | "room" => "here".to_string(),
        "world" => "world".to_string(),
        "avatar" | "me" | "self" => "avatar".to_string(),
        _ => target.trim().to_string(),
    }
}

fn split_target_path(target_token: &str) -> (String, Option<String>) {
    let token = target_token.trim();
    if token.is_empty() {
        return (String::new(), None);
    }
    if let Some((base, tail)) = token.split_once('.') {
        let path = tail.trim();
        if !path.is_empty() {
            return (base.trim().to_string(), Some(path.to_string()));
        }
        return (base.trim().to_string(), None);
    }
    (token.to_string(), None)
}

fn canonical_room_command(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or_default();
    let tail = parts.next().unwrap_or_default().trim();
    let canonical_head = match head.to_ascii_lowercase().as_str() {
        "actors" => "who".to_string(),
        _ => head.to_string(),
    };

    if tail.is_empty() {
        canonical_head
    } else {
        format!("{canonical_head} {tail}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageEnvelope {
    Chatter { text: String },
    RoomCommand { command: String },
    ActorCommand { target: String, command: ActorCommand },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActorCommand {
    Say { payload: String },
    Raw { command: String },
}

pub fn parse_message(input: &str) -> MessageEnvelope {
    let trimmed = input.trim();

    // @@ is shorthand for @world world-admin commands.
    if let Some(after_at2) = trimmed.strip_prefix("@@") {
        let cmd = after_at2.trim().to_string();
        return MessageEnvelope::ActorCommand {
            target: "world".to_string(),
            command: ActorCommand::Raw {
                command: if cmd.is_empty() { "help".to_string() } else { cmd },
            },
        };
    }

    if let Some(after_at) = trimmed.strip_prefix('@') {
        let rest = after_at.trim_start();
        if rest.is_empty() {
            return MessageEnvelope::RoomCommand {
                command: "help".to_string(),
            };
        }

        let mut parts = rest.splitn(2, char::is_whitespace);
        let target_token = parts.next().unwrap_or_default().trim();
        let command = parts.next().unwrap_or_default().trim().to_string();

        if target_token.is_empty() {
            return MessageEnvelope::RoomCommand { command };
        }

        let (target_base, target_path) = split_target_path(target_token);
        let target = canonical_target(&target_base);

        if let Some(path) = target_path {
            // Dotted target paths map to attribute access on the target actor.
            let attribute_cmd = if command.is_empty() {
                format!("prop {}", path)
            } else {
                format!("prop {} {}", path, command)
            };
            return MessageEnvelope::ActorCommand {
                target,
                command: ActorCommand::Raw {
                    command: attribute_cmd,
                },
            };
        }

        if target == "here" {
            return MessageEnvelope::ActorCommand {
                target,
                command: ActorCommand::Raw {
                    command: canonical_room_command(&command),
                },
            };
        }

        if command.is_empty() && (target == "world" || target == "avatar") {
            return MessageEnvelope::ActorCommand {
                target,
                command: ActorCommand::Raw {
                    command: "prop _list".to_string(),
                },
            };
        }

        return MessageEnvelope::ActorCommand {
            target,
            command: parse_actor_command(&command),
        };
    }

    // A leading single-quote is shorthand for `say`.
    if let Some(speech) = trimmed.strip_prefix('\'') {
        return MessageEnvelope::ActorCommand {
            target: "avatar".to_string(),
            command: ActorCommand::Say {
                payload: speech.to_string(),
            },
        };
    }

    // Bare input is interpreted as command to caller avatar.
    MessageEnvelope::ActorCommand {
        target: "avatar".to_string(),
        command: parse_actor_command(trimmed),
    }
}

pub fn parse_actor_command(command: &str) -> ActorCommand {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix("say") {
        if rest.starts_with(char::is_whitespace) {
            return ActorCommand::Say {
                payload: rest.trim().to_string(),
            };
        }
    }

    ActorCommand::Raw {
        command: trimmed.to_string(),
    }
}

pub fn normalize_spoken_text(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() >= 2 {
        let quoted_double = trimmed.starts_with('"') && trimmed.ends_with('"');
        let quoted_single = trimmed.starts_with('\'') && trimmed.ends_with('\'');
        if quoted_double || quoted_single {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }
    trimmed.to_string()
}
