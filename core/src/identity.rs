use did_ma::{Did, Document, EncryptionKey, Result, SigningKey, VerificationMethod};

#[derive(Debug, Clone)]
pub struct GeneratedAgentIdentity {
    pub root_did: Did,
    pub document: Document,
    pub signing_private_key: [u8; 32],
    pub encryption_private_key: [u8; 32],
}

pub fn create_agent_identity_from_private_keys(
    ipns: &str,
    fragment: &str,
    signing_private_key: [u8; 32],
    encryption_private_key: [u8; 32],
) -> Result<GeneratedAgentIdentity> {
    let root_did = Did::new(ipns, fragment)?;
    let sign_did = Did::new(ipns, fragment)?;
    let enc_did = Did::new(ipns, fragment)?;

    let signing_key = SigningKey::from_private_key_bytes(sign_did, signing_private_key)?;
    let encryption_key = EncryptionKey::from_private_key_bytes(enc_did, encryption_private_key)?;

    let mut document = Document::new(&root_did, &root_did);

    let assertion_vm = VerificationMethod::new(
        root_did.base_id(),
        root_did.id(),
        signing_key.key_type.clone(),
        signing_key.did.fragment.as_deref().unwrap_or_default(),
        signing_key.public_key_multibase.clone(),
    )?;

    let key_agreement_vm = VerificationMethod::new(
        root_did.base_id(),
        root_did.id(),
        encryption_key.key_type.clone(),
        encryption_key.did.fragment.as_deref().unwrap_or_default(),
        encryption_key.public_key_multibase.clone(),
    )?;

    let assertion_vm_id = assertion_vm.id.clone();
    document.add_verification_method(assertion_vm.clone())?;
    document.add_verification_method(key_agreement_vm.clone())?;
    document.assertion_method = assertion_vm_id;
    document.key_agreement = key_agreement_vm.id.clone();
    document.set_ma_type("agent")?;
    document.sign(&signing_key, &assertion_vm)?;

    Ok(GeneratedAgentIdentity {
        root_did,
        document,
        signing_private_key,
        encryption_private_key,
    })
}

pub fn create_agent_identity(ipns: &str, fragment: &str) -> Result<GeneratedAgentIdentity> {
    let sign_did = Did::new(ipns, fragment)?;
    let enc_did = Did::new(ipns, fragment)?;
    let signing_key = SigningKey::generate(sign_did)?;
    let encryption_key = EncryptionKey::generate(enc_did)?;
    create_agent_identity_from_private_keys(
        ipns,
        fragment,
        signing_key.private_key_bytes(),
        encryption_key.private_key_bytes(),
    )
}