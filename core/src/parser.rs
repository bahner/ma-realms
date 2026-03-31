use crate::locale::LocaleLexicon;
use serde::{Deserialize, Serialize};

fn canonical_target(target: &str, lexicon: &LocaleLexicon) -> String {
    let normalized = target.trim().to_ascii_lowercase();
    if lexicon.here_aliases.iter().any(|alias| alias == &normalized) {
        return "here".to_string();
    }
    if lexicon.avatar_aliases.iter().any(|alias| alias == &normalized) {
        return "avatar".to_string();
    }
    target.trim().to_string()
}

fn canonical_room_command(command: &str, lexicon: &LocaleLexicon) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or_default();
    let tail = parts.next().unwrap_or_default().trim();
    let canonical_head = lexicon
        .room_command_aliases
        .get(&head.to_ascii_lowercase())
        .cloned()
        .unwrap_or_else(|| head.to_string());

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
    let lexicon = LocaleLexicon::for_locale("en");
    parse_message_with_lexicon(input, &lexicon)
}

pub fn parse_message_with_locale(input: &str, locale: &str) -> MessageEnvelope {
    let lexicon = LocaleLexicon::for_locale(locale);
    parse_message_with_lexicon(input, &lexicon)
}

pub fn parse_message_with_lexicon(input: &str, lexicon: &LocaleLexicon) -> MessageEnvelope {
    let trimmed = input.trim();
    // @@ is shorthand for @world — world-admin commands.
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
        let target = parts.next().unwrap_or_default().trim();
        let command = parts.next().unwrap_or_default().trim().to_string();

        if target.is_empty() {
            return MessageEnvelope::RoomCommand { command };
        }

        let target = canonical_target(target, lexicon);
        if target == "here" {
            return MessageEnvelope::ActorCommand {
                target,
                command: ActorCommand::Raw {
                    command: canonical_room_command(&command, lexicon),
                },
            };
        }

        return MessageEnvelope::ActorCommand {
            target,
            command: parse_actor_command_with_lexicon(&command, lexicon),
        };
    }

    // A leading single-quote is shorthand for `say`: 'Hello → @avatar say Hello
    if let Some(speech) = trimmed.strip_prefix('\'') {
        return MessageEnvelope::ActorCommand {
            target: "avatar".to_string(),
            command: ActorCommand::Say {
                payload: speech.to_string(),
            },
        };
    }

    // Bare input (no @ or ') is a command to the caller's own avatar,
    // parsed through the full lexicon so `say foo` → Say, not Raw.
    MessageEnvelope::ActorCommand {
        target: "avatar".to_string(),
        command: parse_actor_command_with_lexicon(trimmed, lexicon),
    }
}

pub fn parse_actor_command(command: &str) -> ActorCommand {
    let lexicon = LocaleLexicon::for_locale("en");
    parse_actor_command_with_lexicon(command, &lexicon)
}

pub fn parse_actor_command_with_locale(command: &str, locale: &str) -> ActorCommand {
    let lexicon = LocaleLexicon::for_locale(locale);
    parse_actor_command_with_lexicon(command, &lexicon)
}

pub fn parse_actor_command_with_lexicon(command: &str, lexicon: &LocaleLexicon) -> ActorCommand {
    let trimmed = command.trim();
    for verb in &lexicon.say_verbs {
        if let Some(rest) = trimmed.strip_prefix(verb.as_str()) {
            if rest.starts_with(char::is_whitespace) {
                return ActorCommand::Say {
                    payload: rest.trim().to_string(),
                };
            }
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
