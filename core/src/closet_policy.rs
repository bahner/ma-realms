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

    match was_published_before {
        Some(true) => {}
        _ => {
            return Err(anyhow!(
                "ipns_private_key_base64 is required for first DID publish in closet session"
            ));
        }
    }

    let Some(key_name) = local_key_name else {
        return Err(anyhow!(
            "no local IPNS key found for DID root '{}'; provide ipns_private_key_base64",
            document_root
        ));
    };

    Ok(ClosetDidPublishPlan::UseExistingLocalKey { key_name })
}
