#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosetCommand {
    Empty,
    Help,
    Show,
    Hear,
    MissingFieldValue {
        field: ClosetProfileField,
    },
    SetField {
        field: ClosetProfileField,
        value: String,
    },
    Apply {
        ipns_private_key_base64: String,
    },
    Recovery(ClosetRecoveryCommand),
    Enter {
        room: Option<String>,
    },
    Unknown {
        verb: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosetProfileField {
    Name,
    Description,
}

impl ClosetProfileField {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Description => "description",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosetRecoveryCommand {
    Usage,
    Set { passphrase: String },
    Status,
    Rekey { handle: String, passphrase: String },
}

pub const CLOSET_HELP_MESSAGE: &str = "Closet commands: help | show | hear | apply [ipns_key_base64] | citizen [ipns_key_base64] | name <text> | description <text> | recovery set <passphrase> | recovery status | recovery rekey <@handle> <passphrase>\nIf actor DID does not exist yet: run apply first. After actor is created, set avatar name/description.\nFragment is requested by the actor during DID publish; if occupied, retry with another fragment.\nWhen done: type 'go out' in the actor UI to leave the closet.";

pub const CLOSET_HELP_PROMPT: &str = "If actor DID does not exist yet: run apply first. After actor is created, set avatar name/description. Then type 'go out' in the actor UI.";

pub const CLOSET_EMPTY_MESSAGE: &str = "You are in the closet and have no avatar yet. Type 'help'.";

pub fn parse_closet_command(input: &str) -> ClosetCommand {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return ClosetCommand::Empty;
    }

    let mut parts = trimmed.split_whitespace();
    let verb = parts.next().unwrap_or_default().to_ascii_lowercase();
    let tail = trimmed[verb.len()..].trim();

    match verb.as_str() {
        "help" => ClosetCommand::Help,
        "show" | "status" | "look" => ClosetCommand::Show,
        "hear" => ClosetCommand::Hear,
        "name" => {
            if tail.is_empty() {
                ClosetCommand::MissingFieldValue {
                    field: ClosetProfileField::Name,
                }
            } else {
                ClosetCommand::SetField {
                    field: ClosetProfileField::Name,
                    value: tail.to_string(),
                }
            }
        }
        "description" => {
            if tail.is_empty() {
                ClosetCommand::MissingFieldValue {
                    field: ClosetProfileField::Description,
                }
            } else {
                ClosetCommand::SetField {
                    field: ClosetProfileField::Description,
                    value: tail.to_string(),
                }
            }
        }
        "apply" | "citizen" => ClosetCommand::Apply {
            ipns_private_key_base64: tail.to_string(),
        },
        "recovery" => {
            let mut args = tail.split_whitespace();
            let action = args.next().unwrap_or_default().to_ascii_lowercase();
            match action.as_str() {
                "" => ClosetCommand::Recovery(ClosetRecoveryCommand::Usage),
                "set" => {
                    let passphrase = tail.strip_prefix("set").unwrap_or_default().trim();
                    ClosetCommand::Recovery(ClosetRecoveryCommand::Set {
                        passphrase: passphrase.to_string(),
                    })
                }
                "status" => ClosetCommand::Recovery(ClosetRecoveryCommand::Status),
                "rekey" => {
                    let mut split = tail
                        .strip_prefix("rekey")
                        .unwrap_or_default()
                        .trim()
                        .splitn(2, char::is_whitespace);
                    let handle = split.next().unwrap_or_default().trim().to_string();
                    let passphrase = split.next().unwrap_or_default().trim().to_string();
                    ClosetCommand::Recovery(ClosetRecoveryCommand::Rekey { handle, passphrase })
                }
                _ => ClosetCommand::Recovery(ClosetRecoveryCommand::Usage),
            }
        }
        "enter" => {
            let room = if tail.is_empty() {
                None
            } else {
                Some(tail.to_string())
            };
            ClosetCommand::Enter { room }
        }
        _ => ClosetCommand::Unknown { verb },
    }
}

#[cfg(test)]
mod tests {
    use super::{ClosetCommand, ClosetProfileField, ClosetRecoveryCommand, parse_closet_command};

    #[test]
    fn parses_apply_alias() {
        assert_eq!(
            parse_closet_command("citizen Zm9v"),
            ClosetCommand::Apply {
                ipns_private_key_base64: "Zm9v".to_string(),
            }
        );
    }

    #[test]
    fn parses_missing_name_value() {
        assert_eq!(
            parse_closet_command("name"),
            ClosetCommand::MissingFieldValue {
                field: ClosetProfileField::Name,
            }
        );
    }

    #[test]
    fn parses_recovery_rekey() {
        assert_eq!(
            parse_closet_command("recovery rekey @bahner hemmelig"),
            ClosetCommand::Recovery(ClosetRecoveryCommand::Rekey {
                handle: "@bahner".to_string(),
                passphrase: "hemmelig".to_string(),
            })
        );
    }
}
