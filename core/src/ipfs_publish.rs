//! Reusable IPFS DID-document publish service.
//!
//! Extracted from world so that any runtime (world, agent, headless actor)
//! can offer `ma/ipfs/1` without depending on the world crate.

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use did_ma::{Did, Document, Message};
use std::time::Duration;

use crate::protocol::{
    CONTENT_TYPE_DOC, IpfsPublishDidRequest, IpfsPublishDidResponse,
};
use ma_core::kubo::{
    IpnsPublishOptions, dag_put, import_key, list_keys, name_publish_with_retry,
};

/// The validated artefacts from an incoming `ma/ipfs/1` request.
///
/// Returned by [`validate_ipfs_publish_request`] so the caller can
/// inspect the document (e.g. update a DID cache) before publishing.
pub struct ValidatedIpfsPublish {
    pub request: IpfsPublishDidRequest,
    pub document: Document,
    pub document_did: Did,
}

/// Parse, authenticate and validate an IPFS publish request message.
///
/// Does **not** touch Kubo — this is pure validation suitable for
/// gating / caching before the actual publish.
pub fn validate_ipfs_publish_request(
    message_cbor: &[u8],
) -> Result<ValidatedIpfsPublish> {
    let message = Message::from_cbor(message_cbor)
        .map_err(|e| anyhow!("invalid signed message: {}", e))?;

    if message.content_type != CONTENT_TYPE_DOC {
        return Err(anyhow!(
            "expected {} on ma/ipfs/1, got {}",
            CONTENT_TYPE_DOC,
            message.content_type
        ));
    }

    let sender_did = Did::try_from(message.from.as_str())
        .map_err(|e| anyhow!("invalid sender did '{}': {}", message.from, e))?;

    let request: IpfsPublishDidRequest = serde_json::from_slice(&message.content)
        .map_err(|e| anyhow!("invalid IPFS publish payload: {}", e))?;

    let document = Document::unmarshal(&request.did_document_json)
        .map_err(|e| anyhow!("invalid DID document JSON: {}", e))?;
    document
        .validate()
        .map_err(|e| anyhow!("invalid DID document: {}", e))?;
    document
        .verify()
        .map_err(|e| anyhow!("DID document signature verification failed: {}", e))?;

    let document_did = Did::try_from(document.id.as_str())
        .map_err(|e| anyhow!("invalid document DID '{}': {}", document.id, e))?;

    if document_did.ipns != sender_did.ipns {
        return Err(anyhow!(
            "sender IPNS '{}' does not match document IPNS '{}'",
            sender_did.ipns,
            document_did.ipns
        ));
    }

    message
        .verify_with_document(&document)
        .map_err(|e| anyhow!("request signature verification failed: {}", e))?;

    Ok(ValidatedIpfsPublish {
        request,
        document,
        document_did,
    })
}

/// Publish a DID document to Kubo, importing the IPNS key if needed.
///
/// Returns `(key_name, cid)` on success.
pub async fn publish_did_document_to_kubo(
    kubo_url: &str,
    did_document_json: &str,
    ipns_private_key_base64: &str,
    desired_fragment: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    let document = Document::unmarshal(did_document_json)
        .map_err(|e| anyhow!("invalid DID document JSON: {}", e))?;
    let document_did = Did::try_from(document.id.as_str())
        .map_err(|e| anyhow!("invalid document DID '{}': {}", document.id, e))?;
    let document_ipns_id = document_did.ipns.clone();

    let keys = list_keys(kubo_url).await?;

    let desired = desired_fragment
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or(document_did.fragment.clone());

    let mut key_name: Option<String> = None;

    if let Some(alias) = desired {
        if let Some(existing) = keys.iter().find(|k| k.name == alias) {
            if existing.id.trim() != document_ipns_id {
                return Err(anyhow!(
                    "fragment '{}' exists already with another key id",
                    alias
                ));
            }
            key_name = Some(alias);
        } else if !ipns_private_key_base64.trim().is_empty() {
            let key_bytes = B64
                .decode(ipns_private_key_base64.trim())
                .map_err(|e| anyhow!("invalid base64 key payload: {}", e))?;
            let imported = import_key(kubo_url, &alias, key_bytes).await?;
            if imported.id.trim() != document_ipns_id {
                return Err(anyhow!(
                    "imported key id '{}' does not match document ipns '{}'",
                    imported.id,
                    document_ipns_id
                ));
            }
            key_name = Some(alias);
        }
    }

    if key_name.is_none() {
        key_name = keys
            .iter()
            .find(|k| k.id.trim() == document_ipns_id)
            .map(|k| k.name.clone());
    }

    let Some(key_name) = key_name else {
        return Err(anyhow!(
            "no matching Kubo key for DID ipns '{}' and no importable private key provided",
            document_ipns_id
        ));
    };

    let document_cid = dag_put(kubo_url, &document).await?;
    let ipns_options = IpnsPublishOptions::default();
    name_publish_with_retry(
        kubo_url,
        &key_name,
        &document_cid,
        &ipns_options,
        3,
        Duration::from_millis(1_000),
    )
    .await?;

    Ok((Some(key_name), Some(document_cid)))
}

/// Full pipeline: validate request message, then publish to Kubo.
///
/// Convenience for runtimes that don't need to inspect the validated
/// document before publishing (no DID cache, no lock gate, etc.).
pub async fn handle_ipfs_publish(
    kubo_url: &str,
    message_cbor: &[u8],
) -> Result<IpfsPublishDidResponse> {
    let validated = validate_ipfs_publish_request(message_cbor)?;

    let (key_name, cid) = publish_did_document_to_kubo(
        kubo_url,
        &validated.request.did_document_json,
        &validated.request.ipns_private_key_base64,
        validated.request.desired_fragment.as_deref(),
    )
    .await?;

    Ok(IpfsPublishDidResponse {
        ok: true,
        message: "did document published via ma/ipfs/1".to_string(),
        did: Some(validated.document_did.id()),
        key_name,
        cid,
    })
}
