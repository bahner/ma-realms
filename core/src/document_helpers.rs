use anyhow::{Result, anyhow};
use did_ma::Document;

use crate::resolve_inbox_endpoint_id;

pub fn sender_profile_from_document(document: &Document) -> String {
    if let Some(ma) = document.ma.as_ref() {
        if let Some(language) = ma.language.as_ref() {
            let normalized = language.trim();
            if !normalized.is_empty() {
                return normalized.to_string();
            }
        }
    }
    "und".to_string()
}

pub fn normalize_language_for_did_document(language_order: &str) -> Option<String> {
    let tokens = language_order
        .split(|ch: char| ch == ':' || ch == ';' || ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(":"))
    }
}

pub fn sender_push_endpoint_from_document(document: &Document) -> Option<String> {
    let ma = document.ma.as_ref()?;
    let endpoint = resolve_inbox_endpoint_id(
        ma.current_inbox.as_deref(),
        ma.presence_hint.as_deref(),
        ma.transports.as_ref(),
    )?;
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn sender_encryption_pubkey_multibase_from_document(document: &Document) -> Result<String> {
    let ka_id = document.key_agreement.first()
        .ok_or_else(|| anyhow!("sender DID document has no keyAgreement"))?;
    let vm = document
        .get_verification_method_by_id(ka_id)
        .map_err(|e| anyhow!("sender DID document missing keyAgreement verification method: {}", e))?;
    let key = vm.public_key_multibase.trim();
    if key.is_empty() {
        return Err(anyhow!("sender DID document keyAgreement publicKeyMultibase is empty"));
    }
    Ok(key.to_string())
}

pub fn extract_did_description_from_json(document_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(document_json).ok()?;

    let ma_desc = value
        .get("ma")
        .and_then(|ma| ma.get("description"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    if ma_desc.is_some() {
        return ma_desc;
    }

    let top_desc = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    if top_desc.is_some() {
        return top_desc;
    }

    value
        .get("profile")
        .and_then(|profile| profile.get("description"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_language_colon_separated() {
        assert_eq!(
            normalize_language_for_did_document("nb_NO:en_UK"),
            Some("nb_NO:en_UK".to_string())
        );
    }

    #[test]
    fn normalize_language_comma_separated() {
        assert_eq!(
            normalize_language_for_did_document("nb_NO, en_UK"),
            Some("nb_NO:en_UK".to_string())
        );
    }

    #[test]
    fn normalize_language_empty() {
        assert_eq!(normalize_language_for_did_document(""), None);
        assert_eq!(normalize_language_for_did_document("  "), None);
    }

    #[test]
    fn extract_description_from_ma() {
        let json = r#"{"ma":{"description":"Hello world"}}"#;
        assert_eq!(extract_did_description_from_json(json), Some("Hello world".to_string()));
    }

    #[test]
    fn extract_description_from_top() {
        let json = r#"{"description":"Top level"}"#;
        assert_eq!(extract_did_description_from_json(json), Some("Top level".to_string()));
    }

    #[test]
    fn extract_description_from_profile() {
        let json = r#"{"profile":{"description":"Profile desc"}}"#;
        assert_eq!(extract_did_description_from_json(json), Some("Profile desc".to_string()));
    }

    #[test]
    fn extract_description_none() {
        let json = r#"{"name":"foo"}"#;
        assert_eq!(extract_did_description_from_json(json), None);
    }
}
