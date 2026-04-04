use anyhow::{Result, anyhow};
use ma_core::{ObjectDefinition, RequirementSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentType {
    Room,
    Evaluator,
    Avatar,
    World,
    Object,
}

impl ContentType {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "room" => Ok(Self::Room),
            "evaluator" => Ok(Self::Evaluator),
            "avatar" => Ok(Self::Avatar),
            "world" => Ok(Self::World),
            "object" => Ok(Self::Object),
            other => Err(anyhow!(
                "unsupported content type '{}'; expected one of: room, evaluator, avatar, world, object",
                other
            )),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TypedContentEnvelope {
    kind: String,
    version: u32,
    #[serde(rename = "type")]
    content_type: String,
    content: Value,
}

pub fn parse_object_definition_text(raw: &str, cid: &str) -> Result<ObjectDefinition> {
    let doc: TypedContentEnvelope = serde_yaml::from_str(raw).map_err(|e| {
        anyhow!(
            "invalid object content at CID {}: expected typed YAML envelope (kind=/ma/realms/1, type=object): {}",
            cid,
            e
        )
    })?;
    parse_typed_object_envelope(doc, cid)
}

fn parse_typed_object_envelope(doc: TypedContentEnvelope, cid: &str) -> Result<ObjectDefinition> {
    if doc.kind != "/ma/realms/1" {
        return Err(anyhow!(
            "unsupported typed content envelope at CID {}: expected kind=/ma/realms/1",
            cid
        ));
    }

    let content_type = ContentType::parse(&doc.content_type)?;
    if content_type != ContentType::Object {
        return Err(anyhow!(
            "content type '{}' cannot be loaded as object definition at CID {}",
            doc.content_type,
            cid
        ));
    }

    let definition: ObjectDefinition = serde_json::from_value(doc.content)
        .map_err(|e| anyhow!("invalid object content at CID {}: {}", cid, e))?;
    validate_object_definition(&definition, cid)?;
    Ok(definition)
}

pub fn validate_object_definition(definition: &ObjectDefinition, cid: &str) -> Result<()> {
    if definition.verbs.is_empty() {
        return Ok(());
    }

    let has_help = definition.verbs.iter().any(|verb| {
        verb.name.trim().eq_ignore_ascii_case("help")
            || verb
                .aliases
                .iter()
                .any(|alias| alias.trim().eq_ignore_ascii_case("help"))
    });

    if !has_help {
        return Err(anyhow!(
            "object definition at CID {} declares methods but no help verb/alias",
            cid
        ));
    }

    for verb in &definition.verbs {
        if verb.requirements.is_empty() {
            continue;
        }
        let set = RequirementSet::parse_many(&verb.requirements).map_err(|e| {
            anyhow!(
                "invalid requirements in object definition {} for verb '{}': {}",
                cid,
                verb.name,
                e
            )
        })?;
        let report = set.validate();
        if !report.is_ok() {
            let first = report
                .issues
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "unknown requirements validation error".to_string());
            return Err(anyhow!(
                "invalid requirements in object definition {} for verb '{}': {}",
                cid,
                verb.name,
                first
            ));
        }
    }

    Ok(())
}
