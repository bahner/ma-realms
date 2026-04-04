pub mod addressing;
pub mod bootstrap_identity;
pub mod closet_client;
pub mod closet_domain;
pub mod closet_policy;
pub mod capability_acl;
pub mod domain;
pub mod interfaces;
pub mod object_runtime;
pub mod parser;
pub mod protocol;
pub mod requirements;
pub mod room_actor;

pub use addressing::{
    did_root, find_alias_for_address, find_did_by_endpoint, humanize_identifier, humanize_text,
    normalize_endpoint_id, normalize_iroh_address, normalize_relay_url, resolve_alias_input,
    endpoint_id_from_address, endpoint_id_from_transport_value, resolve_inbox_endpoint_id,
};
#[cfg(not(target_arch = "wasm32"))]
pub use bootstrap_identity::{default_ma_config_root, ensure_local_ipns_key_file};
pub use capability_acl::{
    CapabilityAcl, CompiledCapabilityAcl, CompiledSubjectAcl,
    capability_pattern_matches, compile_acl, compile_acl_from_text,
    evaluate_compiled_acl, evaluate_compiled_acl_with_owner,
    parse_capability_acl_text, parse_object_local_capability_acl,
    subject_has_capability, subject_has_capability_with_owner, validate_capability_acl,
};
pub use closet_client::{
    closet_command, closet_start, closet_submit_citizenship, send_closet_request,
};
pub use closet_domain::{
    CLOSET_EMPTY_MESSAGE,
    CLOSET_HELP_MESSAGE,
    CLOSET_HELP_PROMPT,
    CLOSET_REQUIRED_FIELDS_MESSAGE,
    CLOSET_REQUIRED_FIELDS_PROMPT,
    ClosetCommand,
    ClosetProfileField,
    ClosetRecoveryCommand,
    parse_closet_command,
    required_profile_fields_missing,
};
pub use closet_policy::{
    ClosetDidPublishPlan,
    ensure_issued_document_root_match,
    ensure_session_document_root_match,
    plan_closet_did_publish,
};
pub use domain::{ActorType, AvatarActor, ExitData, ObjectData, RoomActor, WorldActor};
pub use interfaces::{AclRuntime, DidPublisher, IpfsPublisher};
pub use object_runtime::{
    CLOSET_COMMANDS_INLINE, MAILBOX_COMMANDS_INLINE,
    ObjectCommandOutput, ObjectCommandResult, ObjectDefinition, ObjectInboxMessage,
    ObjectMessageIntent, ObjectMessageKind, ObjectMessageRetention, ObjectMessageTarget, ObjectPersistencePolicy,
    ObjectProgramRef, ObjectReceiverListener, ObjectRuntimeState, PendingEphemeralRequest,
    ObjectVerbDefinition, ObjectVerbEvaluator,
};
pub use parser::{
    ActorCommand, MessageEnvelope, normalize_spoken_text, parse_actor_command, parse_message,
};
pub use protocol::{
    ClosetRequest, ClosetResponse,
    LaneCapability, PresenceAvatar, RoomEvent, TransportAck, TransportAckCode, WorldCommand,
    WorldLane, WorldRequest, WorldResponse,
    BROADCAST_ALPN, CHAT_ALPN, CLOSET_ALPN, CMD_ALPN, DEFAULT_WORLD_RELAY_URL, INBOX_ALPN, PRESENCE_ALPN,
    WHISPER_ALPN, WORLD_ALPN,
    DEFAULT_CONTENT_TYPE, CONTENT_TYPE_CHAT, CONTENT_TYPE_PRESENCE,
    CONTENT_TYPE_CMD, CONTENT_TYPE_WORLD, CONTENT_TYPE_BROADCAST,
    CONTENT_TYPE_DOC, CONTENT_TYPE_WHISPER,
};
pub use requirements::{
    LegacyRequirement, RequirementArgArity, RequirementChecker, RequirementEvaluation,
    RequirementValue,
    RequirementSet, RequirementSignature, RequirementSpec,
    RequirementValidationIssue, RequirementValidationIssueKind, RequirementValidationReport,
    REQUIREMENT_SIGNATURES, evaluate_requirements, requirement_catalog, validate_requirements,
};
pub use room_actor::{
    execute_room_actor_command, RoomActorAction, RoomActorContext, RoomActorResult,
};

#[cfg(test)]
mod tests {
    use super::{
        ActorCommand, MessageEnvelope, did_root,
        find_alias_for_address, find_did_by_endpoint, humanize_identifier, humanize_text,
        normalize_endpoint_id, normalize_spoken_text, parse_message, resolve_alias_input,
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
