pub mod addressing;
pub mod domain;
pub mod interfaces;
pub mod locale;
pub mod object_runtime;
pub mod parser;
pub mod protocol;
pub mod room_actor;

pub use addressing::{
    did_root, find_alias_for_address, find_did_by_endpoint, humanize_identifier, humanize_text,
    normalize_endpoint_id, normalize_iroh_address, normalize_relay_url, resolve_alias_input,
    endpoint_id_from_address, endpoint_id_from_transport_value, resolve_inbox_endpoint_id,
};
pub use domain::{ActorType, AvatarActor, ExitData, ObjectData, RoomActor, WorldActor};
pub use interfaces::{AclRuntime, DidPublisher, IpfsPublisher};
pub use locale::{LocaleLexicon, canonical_locale, localized_here_alias, localized_say_verb};
pub use object_runtime::{
    ObjectCommandOutput, ObjectCommandResult, ObjectDefinition, ObjectInboxMessage,
    ObjectMessageIntent, ObjectMessageKind, ObjectMessageRetention, ObjectMessageTarget, ObjectPersistencePolicy,
    ObjectProgramRef, ObjectReceiverListener, ObjectRuntimeState, PendingEphemeralRequest,
};
pub use parser::{
    ActorCommand, MessageEnvelope, normalize_spoken_text, parse_actor_command,
    parse_actor_command_with_lexicon, parse_actor_command_with_locale, parse_message,
    parse_message_with_lexicon, parse_message_with_locale,
};
pub use protocol::{
    LaneCapability, PresenceAvatar, RoomEvent, TransportAck, TransportAckCode, WorldCommand,
    WorldLane, WorldRequest, WorldResponse,
    BROADCAST_ALPN, CHAT_ALPN, CMD_ALPN, DEFAULT_WORLD_RELAY_URL, INBOX_ALPN, PRESENCE_ALPN,
    WHISPER_ALPN, WORLD_ALPN,
    DEFAULT_CONTENT_TYPE, CONTENT_TYPE_CHAT, CONTENT_TYPE_PRESENCE,
    CONTENT_TYPE_CMD, CONTENT_TYPE_WORLD, CONTENT_TYPE_BROADCAST,
    CONTENT_TYPE_DOC, CONTENT_TYPE_WHISPER,
};
pub use room_actor::{
    execute_room_actor_command, RoomActorAction, RoomActorContext, RoomActorResult,
};

#[cfg(test)]
mod tests {
    use super::{
        ActorCommand, LocaleLexicon, MessageEnvelope, canonical_locale, did_root,
        find_alias_for_address, find_did_by_endpoint, humanize_identifier, humanize_text,
        localized_here_alias, normalize_endpoint_id, normalize_spoken_text, parse_message,
        parse_message_with_lexicon, parse_message_with_locale, resolve_alias_input,
    };
    use std::collections::HashMap;

    #[test]
    fn parses_chatter() {
        // Bare input is a command to the local avatar, parsed through the lexicon.
        assert_eq!(
            parse_message("Hello, world!"),
            MessageEnvelope::ActorCommand {
                target: "avatar".to_string(),
                command: ActorCommand::Raw {
                    command: "Hello, world!".to_string()
                }
            }
        );
    }

    #[test]
    fn parses_chat_with_quote_prefix() {
        // ' is shorthand for @avatar say.
        assert_eq!(
            parse_message("'Hello, world!"),
            MessageEnvelope::ActorCommand {
                target: "avatar".to_string(),
                command: ActorCommand::Say {
                    payload: "Hello, world!".to_string()
                }
            }
        );
    }

    #[test]
    fn parses_bare_say_verb() {
        // `say foo` (bare) routes through the lexicon and becomes Say, not Raw.
        assert_eq!(
            parse_message("say hello"),
            MessageEnvelope::ActorCommand {
                target: "avatar".to_string(),
                command: ActorCommand::Say {
                    payload: "hello".to_string()
                }
            }
        );
    }

    #[test]
    fn parses_chat_preserves_inner_quotes() {
        // Text after ' is the raw payload — inner quotes are untouched.
        assert_eq!(
            parse_message("' abc blåbærsylttøy '''"),
            MessageEnvelope::ActorCommand {
                target: "avatar".to_string(),
                command: ActorCommand::Say {
                    payload: " abc blåbærsylttøy '''".to_string()
                }
            }
        );
    }

        #[test]
        fn parses_world_admin_shorthand() {
            // @@ is shorthand for @world
            assert_eq!(
                parse_message("@@claim"),
                MessageEnvelope::ActorCommand {
                    target: "world".to_string(),
                    command: ActorCommand::Raw {
                        command: "claim".to_string()
                    }
                }
            );
            assert_eq!(
                parse_message("@@dig north to #garden"),
                MessageEnvelope::ActorCommand {
                    target: "world".to_string(),
                    command: ActorCommand::Raw {
                        command: "dig north to #garden".to_string()
                    }
                }
            );
            // bare @@ = help
            assert_eq!(
                parse_message("@@"),
                MessageEnvelope::ActorCommand {
                    target: "world".to_string(),
                    command: ActorCommand::Raw {
                        command: "help".to_string()
                    }
                }
            );
        }

    #[test]
    fn parses_room_command() {
        assert_eq!(
            parse_message("@here who"),
            MessageEnvelope::ActorCommand {
                target: "here".to_string(),
                command: ActorCommand::Raw {
                    command: "who".to_string()
                }
            }
        );
    }

    #[test]
    fn parses_actor_say_command() {
        assert_eq!(
            parse_message("@bahner say \"Hello\""),
            MessageEnvelope::ActorCommand {
                target: "bahner".to_string(),
                command: ActorCommand::Say {
                    payload: "\"Hello\"".to_string()
                }
            }
        );
    }

    #[test]
    fn normalizes_quoted_text() {
        assert_eq!(normalize_spoken_text("\"Hello\""), "Hello");
    }

    #[test]
    fn normalizes_nb_locale_spellings() {
        assert_eq!(canonical_locale("nb_NO.UTF8"), "nb-NO".to_string());
    }

    #[test]
    fn parses_norwegian_here_alias() {
        assert_eq!(
            parse_message_with_locale("@her hvem", "nb-NO"),
            MessageEnvelope::ActorCommand {
                target: "here".to_string(),
                command: ActorCommand::Raw {
                    command: "who".to_string()
                }
            }
        );
    }

    #[test]
    fn parses_avatar_target() {
        assert_eq!(
            parse_message_with_locale("@avatar si hei", "nb-NO"),
            MessageEnvelope::ActorCommand {
                target: "avatar".to_string(),
                command: ActorCommand::Say {
                    payload: "hei".to_string()
                }
            }
        );
    }

    #[test]
    fn supports_custom_lexicon() {
        let lexicon = LocaleLexicon {
            canonical_locale: "xx".to_string(),
            here_aliases: vec!["der".to_string()],
            avatar_aliases: vec!["self".to_string()],
            say_verbs: vec!["speak".to_string()],
            room_command_aliases: HashMap::from([("wer".to_string(), "who".to_string())]),
        };

        assert_eq!(
            parse_message_with_lexicon("@der wer", &lexicon),
            MessageEnvelope::ActorCommand {
                target: "here".to_string(),
                command: ActorCommand::Raw {
                    command: "who".to_string(),
                },
            }
        );

        assert_eq!(
            parse_message_with_lexicon("@self speak hi", &lexicon),
            MessageEnvelope::ActorCommand {
                target: "avatar".to_string(),
                command: ActorCommand::Say {
                    payload: "hi".to_string(),
                },
            }
        );
    }

    #[test]
    fn exposes_localized_aliases() {
        assert_eq!(localized_here_alias("nb-NO"), "her".to_string());
    }

    #[test]
    fn resolves_did_root() {
        assert_eq!(did_root("did:ma:abc#sig"), "did:ma:abc");
    }

    #[test]
    fn normalizes_endpoint_id() {
        let endpoint = "a".repeat(64);
        assert_eq!(normalize_endpoint_id(&format!("/iroh/{endpoint}")), Some(endpoint));
    }

    #[test]
    fn resolves_alias_input_value() {
        let mut aliases = HashMap::new();
        aliases.insert("home".to_string(), "/iroh/0123".to_string());
        assert_eq!(resolve_alias_input("home", &aliases), "/iroh/0123");
    }

    #[test]
    fn finds_alias_for_did_address() {
        let mut aliases = HashMap::new();
        aliases.insert("dancer".to_string(), "did:ma:k51example".to_string());
        assert_eq!(find_alias_for_address("did:ma:k51example#sig", &aliases), Some("dancer".to_string()));
    }

    #[test]
    fn humanizes_identifier_with_alias() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "world-home".to_string(),
            "/iroh/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        );
        assert_eq!(
            humanize_identifier("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &aliases),
            "world-home"
        );
    }

    #[test]
    fn finds_did_by_endpoint_map() {
        let mut map = HashMap::new();
        map.insert(
            "did:ma:k51example".to_string(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        );
        assert_eq!(
            find_did_by_endpoint("/iroh/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", &map),
            Some("did:ma:k51example".to_string())
        );
    }

    #[test]
    fn humanizes_text_tokens() {
        let mut aliases = HashMap::new();
        aliases.insert("dancer".to_string(), "did:ma:k51example".to_string());
        assert_eq!(humanize_text("did:ma:k51example: hello", &aliases), "dancer: hello");
    }
}
