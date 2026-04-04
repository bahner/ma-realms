use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementArgArity {
    None,
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequirementSignature {
    pub name: &'static str,
    pub arg_arity: RequirementArgArity,
}

pub const REQUIREMENT_SIGNATURES: &[RequirementSignature] = &[
    // Object requirements
    RequirementSignature { name: "object.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.in_room", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.held", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.not_held", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.held_by_self", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.held_by_other", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.open", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.closed", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.locked", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.unlocked", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.opened_by_self", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.opened_by_other", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.owned", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.owner_is_self", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.owner_is_world_owner", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.alias_bound", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.method_available", arg_arity: RequirementArgArity::Required },
    RequirementSignature { name: "object.durable", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "object.persistence", arg_arity: RequirementArgArity::Required },
    // Actor/avatar requirements
    RequirementSignature { name: "actor.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.authenticated", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.registered", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.has_citizenship", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.handle_set", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.in_room", arg_arity: RequirementArgArity::Optional },
    RequirementSignature { name: "actor.owner_of_world", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.owner_of_room", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.owner_of_object", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.blocked_by_target", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "actor.not_blocked_by_target", arg_arity: RequirementArgArity::None },
    // Room requirements
    RequirementSignature { name: "room.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "room.in", arg_arity: RequirementArgArity::Optional },
    RequirementSignature { name: "room.owned", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "room.owner_is_self", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "room.private", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "room.public", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "room.has_exit", arg_arity: RequirementArgArity::Required },
    RequirementSignature { name: "room.acl_allows_entry", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "room.acl_allows_action", arg_arity: RequirementArgArity::Required },
    RequirementSignature { name: "room.has_object", arg_arity: RequirementArgArity::Required },
    RequirementSignature { name: "room.object_reachable", arg_arity: RequirementArgArity::Required },
    // World requirements
    RequirementSignature { name: "world.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.unlocked", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.owned", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.owner_is_self", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.entry_allowed", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.private", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.public", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.knock_pending", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.knock_accepted", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.knock_rejected", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "world.protocol_available", arg_arity: RequirementArgArity::Required },
    // Session/transport
    RequirementSignature { name: "session.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "session.closet_active", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "session.world_active", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "session.bound_to_endpoint", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "transport.connected", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "transport.relay_available", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "transport.endpoint_known", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "transport.alpn", arg_arity: RequirementArgArity::Required },
    // Mailbox
    RequirementSignature { name: "mailbox.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.held", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.open", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.opened_by_self", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.owner_only", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.has_pending_knocks", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.has_messages", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "mailbox.request_pending", arg_arity: RequirementArgArity::Required },
    // Knock/citizenship
    RequirementSignature { name: "knock.exists", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "knock.pending", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "knock.accepted", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "knock.rejected", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "knock.target_room", arg_arity: RequirementArgArity::Required },
    RequirementSignature { name: "citizenship.granted", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "citizenship.required", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "citizenship.import_mode", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "citizenship.bootstrap_mode", arg_arity: RequirementArgArity::None },
    // Identity/DID
    RequirementSignature { name: "did.present", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.valid", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.root_matches_session", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.document_available", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.document_signature_valid", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.has_fragment", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.belongs_to_world", arg_arity: RequirementArgArity::None },
    RequirementSignature { name: "did.transport_hint_present", arg_arity: RequirementArgArity::None },
];

const CONTRADICTIONS: &[(&str, &str)] = &[
    ("object.held", "object.not_held"),
    ("object.open", "object.closed"),
    ("object.locked", "object.unlocked"),
    ("object.held_by_self", "object.held_by_other"),
    ("object.opened_by_self", "object.opened_by_other"),
    ("room.private", "room.public"),
    ("world.private", "world.public"),
    ("knock.pending", "knock.accepted"),
    ("knock.pending", "knock.rejected"),
    ("knock.accepted", "knock.rejected"),
    ("world.knock_pending", "world.knock_accepted"),
    ("world.knock_pending", "world.knock_rejected"),
    ("world.knock_accepted", "world.knock_rejected"),
    ("actor.blocked_by_target", "actor.not_blocked_by_target"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
}

impl RequirementSpec {
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("empty requirement".to_string());
        }

        if let Some(open_idx) = trimmed.find('(') {
            if !trimmed.ends_with(')') {
                return Err(format!("invalid requirement '{}': missing closing ')'", trimmed));
            }
            let name = trimmed[..open_idx].trim();
            let arg_raw = &trimmed[open_idx + 1..trimmed.len() - 1];
            let arg = arg_raw.trim();
            if arg.is_empty() {
                return Err(format!("invalid requirement '{}': empty argument", trimmed));
            }
            validate_name(name)?;
            return Ok(Self {
                name: name.to_string(),
                arg: Some(arg.to_string()),
            });
        }

        validate_name(trimmed)?;
        Ok(Self {
            name: trimmed.to_string(),
            arg: None,
        })
    }

    pub fn render(&self) -> String {
        match self.arg.as_ref() {
            Some(arg) => format!("{}({})", self.name, arg),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementValidationIssueKind {
    UnknownRequirement,
    InvalidArgumentArity,
    Contradiction,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementValidationIssue {
    pub kind: RequirementValidationIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementValidationReport {
    pub issues: Vec<RequirementValidationIssue>,
}

impl RequirementValidationReport {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSet {
    pub all_of: Vec<RequirementSpec>,
}

impl RequirementSet {
    pub fn parse_many(items: &[String]) -> Result<Self, String> {
        let mut all_of = Vec::with_capacity(items.len());
        for item in items {
            all_of.push(RequirementSpec::parse(item)?);
        }
        Ok(Self { all_of })
    }

    pub fn validate(&self) -> RequirementValidationReport {
        validate_requirements(&self.all_of)
    }

    pub fn evaluate<C: RequirementChecker>(&self, checker: &C) -> RequirementEvaluation {
        evaluate_requirements(checker, &self.all_of)
    }
}

pub trait RequirementChecker {
    fn check_requirement(&self, requirement: &RequirementSpec) -> bool;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementEvaluation {
    pub passed: bool,
    pub failed: Vec<RequirementSpec>,
}

pub fn requirement_catalog() -> Vec<&'static str> {
    REQUIREMENT_SIGNATURES.iter().map(|sig| sig.name).collect()
}

pub fn validate_requirements(requirements: &[RequirementSpec]) -> RequirementValidationReport {
    let mut issues = Vec::new();

    for requirement in requirements {
        let Some(signature) = REQUIREMENT_SIGNATURES
            .iter()
            .find(|sig| sig.name == requirement.name)
            .copied()
        else {
            issues.push(RequirementValidationIssue {
                kind: RequirementValidationIssueKind::UnknownRequirement,
                message: format!("unknown requirement '{}'", requirement.render()),
            });
            continue;
        };

        let arg_ok = match (signature.arg_arity, requirement.arg.as_ref()) {
            (RequirementArgArity::None, None) => true,
            (RequirementArgArity::None, Some(_)) => false,
            (RequirementArgArity::Optional, _) => true,
            (RequirementArgArity::Required, Some(arg)) => !arg.trim().is_empty(),
            (RequirementArgArity::Required, None) => false,
        };

        if !arg_ok {
            let arity_hint = match signature.arg_arity {
                RequirementArgArity::None => "takes no argument",
                RequirementArgArity::Optional => "takes an optional argument",
                RequirementArgArity::Required => "requires an argument",
            };
            issues.push(RequirementValidationIssue {
                kind: RequirementValidationIssueKind::InvalidArgumentArity,
                message: format!("invalid requirement '{}': {}", requirement.render(), arity_hint),
            });
        }
    }

    for i in 0..requirements.len() {
        for j in (i + 1)..requirements.len() {
            let a = &requirements[i];
            let b = &requirements[j];
            if a.name == b.name && a.arg == b.arg {
                issues.push(RequirementValidationIssue {
                    kind: RequirementValidationIssueKind::Duplicate,
                    message: format!("duplicate requirement '{}'", a.render()),
                });
            }

            if CONTRADICTIONS.iter().any(|(x, y)| {
                (a.name == *x && b.name == *y) || (a.name == *y && b.name == *x)
            }) {
                issues.push(RequirementValidationIssue {
                    kind: RequirementValidationIssueKind::Contradiction,
                    message: format!("contradictory requirements '{}' and '{}'", a.render(), b.render()),
                });
            }
        }
    }

    RequirementValidationReport { issues }
}

pub fn evaluate_requirements<C: RequirementChecker>(
    checker: &C,
    requirements: &[RequirementSpec],
) -> RequirementEvaluation {
    let mut failed = Vec::new();
    for requirement in requirements {
        if !checker.check_requirement(requirement) {
            failed.push(requirement.clone());
        }
    }

    RequirementEvaluation {
        passed: failed.is_empty(),
        failed,
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("requirement name cannot be empty".to_string());
    }

    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '_');
    if !valid {
        return Err(format!(
            "invalid requirement name '{}': only [a-z0-9._] allowed",
            name
        ));
    }

    if !name.contains('.') {
        return Err(format!(
            "invalid requirement name '{}': expected dotted form like domain.subject",
            name
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requirement_with_arg() {
        let req = RequirementSpec::parse("object.method_available(help)").expect("parse ok");
        assert_eq!(req.name, "object.method_available");
        assert_eq!(req.arg.as_deref(), Some("help"));
    }

    #[test]
    fn rejects_unknown_and_contradictory_requirements() {
        let requirements = vec![
            RequirementSpec::parse("object.held").expect("held parse"),
            RequirementSpec::parse("object.not_held").expect("not held parse"),
            RequirementSpec::parse("foo.bar").expect("unknown parses syntactically"),
        ];

        let report = validate_requirements(&requirements);
        assert!(!report.is_ok());
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == RequirementValidationIssueKind::Contradiction));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == RequirementValidationIssueKind::UnknownRequirement));
    }

    struct DummyChecker;

    impl RequirementChecker for DummyChecker {
        fn check_requirement(&self, requirement: &RequirementSpec) -> bool {
            requirement.name != "object.locked"
        }
    }

    #[test]
    fn evaluates_requirement_set() {
        let set = RequirementSet {
            all_of: vec![
                RequirementSpec::parse("object.held").expect("parse held"),
                RequirementSpec::parse("object.locked").expect("parse locked"),
            ],
        };
        let out = set.evaluate(&DummyChecker);
        assert!(!out.passed);
        assert_eq!(out.failed.len(), 1);
        assert_eq!(out.failed[0].name, "object.locked");
    }
}
