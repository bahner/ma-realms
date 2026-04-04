use anyhow::{anyhow, Result};
use did_ma::Did;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosetDidPublishPlan {
    ImportProvidedKey,
    UseExistingLocalKey { key_name: String },
}

pub fn ensure_session_document_root_match(
    session_did: Option<&str>,
    document_root: &str,
) -> Result<()> {
    let Some(session_did) = session_did else {
        return Ok(());
    };

    let session_root = Did::try_from(session_did)
        .map_err(|e| anyhow!("invalid session DID '{}': {}", session_did, e))?
        .without_fragment()
        .id();

    if session_root != document_root {
        return Err(anyhow!(
            "document DID root '{}' does not match session DID root '{}'",
            document_root,
            session_root
        ));
    }

    Ok(())
}

pub fn ensure_issued_document_root_match(issued_did: &str, document_root: &str) -> Result<()> {
    let issued_root = Did::try_from(issued_did)
        .map_err(|e| anyhow!("invalid issued DID '{}': {}", issued_did, e))?
        .without_fragment()
        .id();

    if issued_root != document_root {
        return Err(anyhow!(
            "document DID root '{}' does not match imported key DID root '{}'",
            document_root,
            issued_root
        ));
    }

    Ok(())
}

pub fn plan_closet_did_publish(
    provided_key: &str,
    document_root: &str,
    was_published_before: Option<bool>,
    local_key_name: Option<String>,
) -> Result<ClosetDidPublishPlan> {
    if !provided_key.trim().is_empty() {
        return Ok(ClosetDidPublishPlan::ImportProvidedKey);
    }

    if let Some(key_name) = local_key_name {
        return Ok(ClosetDidPublishPlan::UseExistingLocalKey { key_name });
    }

    match was_published_before {
        Some(true) => {
            return Err(anyhow!(
                "DID appears previously published, but no matching local IPNS key found for DID root '{}'; provide ipns_private_key_base64",
                document_root
            ));
        }
        _ => {
            return Err(anyhow!(
                "ipns_private_key_base64 is required for first DID publish in closet session"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_closet_did_publish, ClosetDidPublishPlan};

    #[test]
    fn uses_existing_local_key_even_if_not_previously_published() {
        let plan = plan_closet_did_publish(
            "",
            "did:ma:k51qzi5uqu5example",
            Some(false),
            Some("bahner".to_string()),
        )
        .expect("expected existing local key to be usable");

        assert_eq!(
            plan,
            ClosetDidPublishPlan::UseExistingLocalKey {
                key_name: "bahner".to_string()
            }
        );
    }

    #[test]
    fn requires_provided_key_for_first_publish_when_no_local_key() {
        let err = plan_closet_did_publish("", "did:ma:k51qzi5uqu5example", Some(false), None)
            .expect_err("expected missing key error");
        assert!(
            err.to_string()
                .contains("ipns_private_key_base64 is required for first DID publish"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn still_prefers_import_when_key_is_provided() {
        let plan = plan_closet_did_publish(
            "Zm9v",
            "did:ma:k51qzi5uqu5example",
            Some(false),
            Some("bahner".to_string()),
        )
        .expect("expected provided key to force import path");

        assert_eq!(plan, ClosetDidPublishPlan::ImportProvidedKey);
    }
}
