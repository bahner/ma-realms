use super::*;

#[derive(Debug, Deserialize)]
struct MaWorldProjectionConfig {
    fields: Vec<String>,
}

fn load_ma_world_projection_fields() -> Result<Vec<String>> {
    const RAW: &str = include_str!("ma_world_projection_fields.yaml");

    let parsed: MaWorldProjectionConfig = serde_yaml::from_str(RAW)
        .map_err(|e| anyhow!("failed to parse ma world projection config: {}", e))?;
    if parsed.fields.is_empty() {
        return Err(anyhow!("ma world projection config must contain at least one field"));
    }

    let valid_fields: HashSet<&str> = [
        "name",
        "world_did",
        "entry_acl",
        "owner",
        "lang_cid",
        "lang",
        "state_cid",
        "public",
        "root",
    ]
    .into_iter()
    .collect();

    let mut selected: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for raw in parsed.fields {
        let input_field = raw.trim().to_string();
        let field = if input_field == "lang_cid" {
            // Projection format is canonical `lang` IPLD link; `lang_cid` is accepted as config alias.
            "lang".to_string()
        } else {
            input_field
        };
        if field.is_empty() {
            return Err(anyhow!("ma world projection config contains an empty field name"));
        }
        if !valid_fields.contains(field.as_str()) {
            return Err(anyhow!(
                "ma world projection config contains invalid field '{}'",
                field
            ));
        }
        if seen.insert(field.clone()) {
            selected.push(field);
        }
    }

    if !selected.iter().any(|field| field == "root") {
        return Err(anyhow!(
            "ma world projection config must include required field 'root'"
        ));
    }
    if !selected.iter().any(|field| field == "public") {
        return Err(anyhow!(
            "ma world projection config must include required field 'public'"
        ));
    }

    Ok(selected)
}

pub(crate) async fn publish_world_did_runtime_ma(
    kubo_url: &str,
    world_slug: &str,
    world_master_key: [u8; 32],
    state_cid: &str,
    root_cid: &str,
    owner_did: Option<String>,
    owner_identity_link: Option<String>,
    entry_acl: String,
    lang_cid: Option<String>,
) -> Result<()> {
    const RUNTIME_IPNS_ATTEMPTS: u32 = 3;
    const RUNTIME_IPNS_TIMEOUT_SECS: u64 = 12;
    const RUNTIME_IPNS_BACKOFF_MS: u64 = 500;

    let world_key_name = normalize_world_key_name(world_slug);
    let did_identifier = ensure_kubo_key_id(kubo_url, &world_key_name).await?;
    let world_did = Did::new(&did_identifier, world_slug)
        .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;
    let world_ipns_path = format!("/ipns/{}", world_did.ipns);

    let signing_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build signing DID: {}", e))?;
    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        derive_world_signing_private_key(&world_master_key),
    )
    .map_err(|e| anyhow!("failed to restore world signing key: {}", e))?;

    let mut document = kubo::fetch_did_document(kubo_url, &world_did).await?;
    document.set_ma_type("world")?;
    set_document_ma_string_field(&mut document, "ipns", &format!("/ipns/{}", world_did.ipns))?;

    let world_root: WorldRootIndexDag = dag_get_dag_cbor(kubo_url, root_cid).await
        .map_err(|e| anyhow!("failed to load world root {} for DID projection: {}", root_cid, e))?;
    let public_cid = dag_put_dag_cbor(kubo_url, &world_root.public).await
        .map_err(|e| anyhow!("failed to publish world public projection for DID: {}", e))?;
    let owner_projection = if let Some(owner_id) = owner_did.as_ref().filter(|v| !v.trim().is_empty()) {
        let owner_ipns = Did::try_from(owner_id.as_str())
            .ok()
            .map(|did| format!("/ipns/{}", did.ipns));
        let owner_identity = owner_identity_link
            .as_ref()
            .map(|link| serde_json::json!({ "/": link }));
        serde_json::json!({
            "id": owner_id,
            "ipns": owner_ipns,
            "identity": owner_identity,
        })
    } else {
        serde_json::Value::Null
    };

    let mut candidates = serde_json::Map::new();
    candidates.insert("name".to_string(), serde_json::Value::String(world_slug.to_string()));
    candidates.insert("world_did".to_string(), serde_json::Value::String(world_did.id()));
    candidates.insert("entry_acl".to_string(), serde_json::Value::String(entry_acl));
    candidates.insert("owner".to_string(), owner_projection);
    let lang_projection = lang_cid
        .filter(|value| !value.trim().is_empty())
        .map(|cid| serde_json::json!({ "/": cid }))
        .unwrap_or(serde_json::Value::Null);
    candidates.insert("lang".to_string(), lang_projection);
    candidates.insert("state_cid".to_string(), serde_json::Value::String(state_cid.to_string()));
    candidates.insert("public".to_string(), serde_json::json!({ "/": public_cid }));
    candidates.insert("root".to_string(), serde_json::json!({ "/": root_cid }));

    let projection_fields = load_ma_world_projection_fields()?;
    let mut ma_world = serde_json::Map::new();
    for field in projection_fields {
        let value = candidates
            .get(&field)
            .cloned()
            .ok_or_else(|| anyhow!("projection field '{}' is unavailable", field))?;
        ma_world.insert(field, value);
    }

    // Publish a tailored world projection in ma.world.
    // Keep `root` link for runtime restore compatibility, while exposing a
    // dedicated `public` link and explicit `owner` metadata for traversal.
    document.set_ma_world(serde_json::Value::Object(ma_world));

    let assertion_id = document.assertion_method.first()
        .ok_or_else(|| anyhow!("world DID has no assertionMethod"))?
        .clone();
    let assertion_vm = document
        .get_verification_method_by_id(&assertion_id)
        .map_err(|e| anyhow!("world DID missing assertion method '{}': {}", assertion_id, e))?
        .clone();
    document.sign(&signing_key, &assertion_vm)?;

    let document_cid = dag_put_dag_cbor(kubo_url, &document).await?;

    let ipns_ttl_secs = std::env::var("MA_WORLD_IPNS_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let ipns_options = IpnsPublishOptions {
        timeout: Duration::from_secs(RUNTIME_IPNS_TIMEOUT_SECS),
        ttl: ipns_ttl_secs.map(|s| format!("{}s", s)),
        ..IpnsPublishOptions::default()
    };
    ipns_publish_with_retry(
        kubo_url,
        &world_key_name,
        &document_cid,
        &ipns_options,
        RUNTIME_IPNS_ATTEMPTS,
        Duration::from_millis(RUNTIME_IPNS_BACKOFF_MS),
    )
    .await?;

    info!(
        "World runtime document available at {} (CID {})",
        world_ipns_path,
        document_cid
    );
    info!("Copy/paste: ipfs dag get {}", world_ipns_path);

    Ok(())
}

async fn ensure_world_did_document(
    kubo_url: &str,
    world_slug: &str,
    endpoint_id: &str,
    world_master_key: [u8; 32],
) -> Result<String> {
    let key_name = normalize_world_key_name(world_slug);
    let mut keys = list_kubo_keys(kubo_url).await?;
    if !keys.iter().any(|key| key.name == key_name) {
        generate_kubo_key(kubo_url, &key_name).await?;
        keys = list_kubo_keys(kubo_url).await?;
    }

    let did_identifier = keys
        .iter()
        .find(|key| key.name == key_name)
        .map(|key| key.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!(
            "kubo key '{}' exists but did not expose a usable Id in key/list",
            key_name
        ))?;

    let world_did = Did::new(&did_identifier, world_slug)
        .map_err(|e| anyhow!("failed to build world DID from IPNS key '{}' slug '{}': {}", did_identifier, world_slug, e))?;
    let world_ipns_path = format!("/ipns/{}", did_identifier);

    let signing_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build signing DID: {}", e))?;
    let signing_key = SigningKey::from_private_key_bytes(
        signing_did,
        derive_world_signing_private_key(&world_master_key),
    )
        .map_err(|e| anyhow!("failed to restore world signing key: {}", e))?;

    let key_agreement_did = Did::new_root(&did_identifier)
        .map_err(|e| anyhow!("failed to build key-agreement DID: {}", e))?;
    let key_agreement_key = EncryptionKey::from_private_key_bytes(
        key_agreement_did,
        derive_world_encryption_private_key(&world_master_key),
    )
    .map_err(|e| anyhow!("failed to restore world key-agreement key: {}", e))?;

    let mut document = Document::new(&world_did, &world_did);

    let assertion_vm = VerificationMethod::new(
        world_did.base_id(),
        world_did.base_id(),
        signing_key.key_type.clone(),
        signing_key.did.fragment.as_deref().unwrap_or_default(),
        signing_key.public_key_multibase.clone(),
    )
    .map_err(|e| anyhow!("failed building world assertion method: {}", e))?;

    let key_agreement_vm = VerificationMethod::new(
        world_did.base_id(),
        world_did.base_id(),
        key_agreement_key.key_type.clone(),
        key_agreement_key.did.fragment.as_deref().unwrap_or_default(),
        key_agreement_key.public_key_multibase.clone(),
    )
    .map_err(|e| anyhow!("failed building world keyAgreement method: {}", e))?;

    let assertion_vm_id = assertion_vm.id.clone();
    let key_agreement_vm_id = key_agreement_vm.id.clone();
    document.add_verification_method(assertion_vm.clone())?;
    document.add_verification_method(key_agreement_vm)?;
    document.assertion_method = vec![assertion_vm_id];
    document.key_agreement = vec![key_agreement_vm_id];
    document.set_ma_type("world")?;
    set_document_ma_string_field(&mut document, "ipns", &format!("/ipns/{}", did_identifier))?;
    let transport_paths = vec![
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(INBOX_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(AVATAR_ALPN)),
        format!("/ma-iroh/{endpoint_id}/{}", String::from_utf8_lossy(IPFS_ALPN)),
    ];
    document.set_ma_transports(serde_json::Value::Array(
        transport_paths
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    ));
    document.set_ma_ping_interval_secs(WORLD_PING_INTERVAL_SECS);
    document.sign(&signing_key, &assertion_vm)?;

    let document_cid = dag_put_dag_cbor(kubo_url, &document).await?;

    info!(
        "World DID document {} stored as CID {} — IPNS publish continues in background",
        world_did.base_id(),
        document_cid
    );
    info!("World document available at {}", world_ipns_path);
    info!("Copy/paste: ipfs dag get {}", world_ipns_path);

    // Spawn IPNS publish in background so startup is not blocked by slow Kubo publishes.
    let bg_kubo_url = kubo_url.to_string();
    let bg_key_name = key_name.clone();
    let bg_document_cid = document_cid.clone();
    let bg_world_did_id = world_did.base_id();
    let bg_world_ipns = world_did.ipns.clone();
    tokio::spawn(async move {
        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_secs(45),
            ..IpnsPublishOptions::default()
        };
        match ipns_publish_with_retry(
            &bg_kubo_url,
            &bg_key_name,
            &bg_document_cid,
            &ipns_options,
            8,
            Duration::from_millis(1500),
        )
        .await
        {
            Ok(_) => info!(
                "Background Identity publication complete for {}: /ipns/{} -> /ipfs/{}",
                bg_world_did_id, bg_world_ipns, bg_document_cid
            ),
            Err(err) => warn!(
                "Background IPNS publish failed for {} CID {}: {}",
                bg_world_did_id, bg_document_cid, err
            ),
        }
    });

    Ok(world_did.base_id())
}

pub(crate) async fn run_main() -> Result<()> {
    let raw_args = std::env::args().collect::<Vec<_>>();
    let args = extract_global_config_arg(raw_args)?;
    let mut run_arg_mode = false;
    let mut listen_addr: String = DEFAULT_LISTEN_ADDR.to_string();
    let mut kubo_url_override: Option<String> = None;
    let mut log_level: String = "info".to_string();
    let mut log_file: Option<PathBuf> = None;
    let mut world_slug_override: Option<String> = None;
    let mut owner_override: Option<String> = None;

    if args.len() >= 2 {
        match args[1].as_str() {
            "-h" | "--help" | "help" => {
                print_cli_help();
                return Ok(());
            }
            "--gen-iroh-secret" => {
                let mut explicit_path: Option<PathBuf> = None;
                let mut world_slug = DEFAULT_WORLD_SLUG.to_string();
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug = args[idx].clone();
                        }
                        other => {
                            if explicit_path.is_some() {
                                return Err(anyhow!(
                                    "usage: ma-world --gen-iroh-secret [path] [--slug <slug>]"
                                ));
                            }
                            explicit_path = Some(PathBuf::from(other));
                        }
                    }
                    idx += 1;
                }

                let normalized_slug = normalize_world_key_name(&world_slug);
                let runtime_cfg_path = runtime_config_path(&normalized_slug);
                let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
                let path = explicit_path
                    .or_else(|| runtime_cfg.iroh_secret.map(PathBuf::from))
                    .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));

                generate_iroh_secret_file(&path)?;
                println!("generated iroh secret: {}", path.display());
                return Ok(());
            }
            "--gen-headless-config" => {
                let mut world_slug: Option<String> = None;
                let mut passphrase: Option<String> = None;
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug = Some(args[idx].clone());
                        }
                        "--passphrase" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --passphrase"));
                            }
                            passphrase = Some(args[idx].clone());
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for --gen-headless-config (supported: --slug, --passphrase)",
                                other
                            ));
                        }
                    }
                    idx += 1;
                }

                let world_slug = world_slug
                    .ok_or_else(|| anyhow!("--slug is required for --gen-headless-config"))?;
                let normalized_slug = normalize_world_key_name(&world_slug);
                let runtime_cfg_path = runtime_config_path(&normalized_slug);
                let cfg_dir = runtime_cfg_path
                    .parent()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow!("invalid runtime config path {}", runtime_cfg_path.display()))?;
                ensure_private_dir(&cfg_dir)?;

                let iroh_path = cfg_dir.join(format!("{}_iroh.bin", normalized_slug));
                let bundle_path = cfg_dir.join(format!("{}_bundle.json", normalized_slug));
                let config_path = cfg_dir.join(format!("{}.yaml", normalized_slug));

                if config_path.exists() {
                    return Err(anyhow!("config already exists at {}", config_path.display()));
                }

                let passphrase = passphrase.unwrap_or_else(|| nanoid!(32));

                generate_iroh_secret_file(&iroh_path)?;

                let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
                    .ok_or_else(|| anyhow!("failed loading generated iroh secret {}", iroh_path.display()))?;
                let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);

                let world = World::new(
                    EntryAcl {
                        allow_all: true,
                        allow_owner: true,
                        allowed_dids: HashSet::new(),
                        source: "*".to_string(),
                    },
                    DEFAULT_KUBO_API_URL.to_string(),
                    normalized_slug.clone(),
                );
                world.set_world_master_key(world_master_key).await;
                let bundle_json = world.create_unlock_bundle(&passphrase).await?;
                write_secure_file(&bundle_path, bundle_json.as_bytes(), SecureFileKind::SensitiveData)?;

                let cfg = bootstrap::RuntimeFileConfig {
                    iroh_secret: Some(iroh_path.display().to_string()),
                    status_api_enabled: Some(true),
                    admin_api_enabled: Some(false),
                    unlock_passphrase: Some(passphrase.clone()),
                    unlock_bundle_file: Some(bundle_path.display().to_string()),
                    ..Default::default()
                };
                let yaml = serde_yaml::to_string(&cfg)?;
                write_secure_file(&config_path, yaml.as_bytes(), SecureFileKind::RuntimeConfig)?;

                println!("generated headless config artifacts for slug '{}':", normalized_slug);
                println!("  iroh_secret: {}", iroh_path.display());
                println!("  unlock_bundle_file: {}", bundle_path.display());
                println!("  config: {}", config_path.display());
                println!("  passphrase: {}", passphrase);
                return Ok(());
            }
            "create-unlock-bundle" => {
                let mut world_slug = DEFAULT_WORLD_SLUG.to_string();
                let mut passphrase: Option<String> = None;
                let mut out_path: Option<PathBuf> = None;
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug = args[idx].clone();
                        }
                        "--passphrase" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --passphrase"));
                            }
                            passphrase = Some(args[idx].clone());
                        }
                        "--out" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --out"));
                            }
                            out_path = Some(PathBuf::from(&args[idx]));
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for create-unlock-bundle (supported: --slug, --passphrase, --out)",
                                other
                            ));
                        }
                    }
                    idx += 1;
                }

                let passphrase = passphrase
                    .ok_or_else(|| anyhow!("--passphrase is required for create-unlock-bundle"))?;
                let normalized_slug = normalize_world_key_name(&world_slug);
                let runtime_cfg_path = runtime_config_path(&normalized_slug);
                let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
                let iroh_path = runtime_cfg
                    .iroh_secret
                    .map(PathBuf::from)
                    .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));
                let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
                    .ok_or_else(|| anyhow!(
                        "missing iroh secret at {}. Create it with: ma-world --gen-iroh-secret --slug {}",
                        iroh_path.display(),
                        normalized_slug
                    ))?;
                let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);

                let world = World::new(
                    EntryAcl {
                        allow_all: true,
                        allow_owner: true,
                        allowed_dids: HashSet::new(),
                        source: "*".to_string(),
                    },
                    DEFAULT_KUBO_API_URL.to_string(),
                    normalized_slug.clone(),
                );
                world.set_world_master_key(world_master_key).await;
                let bundle_json = world.create_unlock_bundle(&passphrase).await?;

                let out_path = out_path.unwrap_or_else(|| {
                    runtime_config_path(&normalized_slug)
                        .with_file_name(format!("{}_bundle.json", normalized_slug))
                });
                write_secure_file(&out_path, bundle_json.as_bytes(), SecureFileKind::SensitiveData)?;
                println!("created unlock bundle: {}", out_path.display());
                return Ok(());
            }
            "run" => {
                run_arg_mode = true;
                let mut idx = 2usize;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--listen" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --listen"));
                            }
                            listen_addr = args[idx].clone();
                        }
                        "--kubo-url" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --kubo-url"));
                            }
                            kubo_url_override = Some(args[idx].clone());
                        }
                        "--owner" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --owner"));
                            }
                            owner_override = Some(args[idx].clone());
                        }
                        "--log-level" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --log-level"));
                            }
                            log_level = args[idx].clone();
                        }
                        "--log-file" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --log-file"));
                            }
                            log_file = Some(expand_tilde_path(&args[idx]));
                        }
                        "--slug" => {
                            idx += 1;
                            if idx >= args.len() {
                                return Err(anyhow!("missing value for --slug"));
                            }
                            world_slug_override = Some(args[idx].clone());
                        }
                        other => {
                            return Err(anyhow!(
                                "unknown argument '{}' for run (supported: --slug, --listen, --kubo-url, --owner, --log-level, --log-file)",
                                other
                            ));
                        }
                    }
                    idx += 1;
                }
            }
            _ => {}
        }
    }

    // Backwards-compatible server mode with top-level flags and no explicit command.
    if !run_arg_mode && args.len() >= 2 && args[1].starts_with('-') {
        run_arg_mode = true;
        let mut idx = 1usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--listen" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --listen"));
                    }
                    listen_addr = args[idx].clone();
                }
                "--kubo-url" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --kubo-url"));
                    }
                    kubo_url_override = Some(args[idx].clone());
                }
                "--owner" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --owner"));
                    }
                    owner_override = Some(args[idx].clone());
                }
                "--log-level" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --log-level"));
                    }
                    log_level = args[idx].clone();
                }
                "--log-file" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --log-file"));
                    }
                    log_file = Some(expand_tilde_path(&args[idx]));
                }
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug_override = Some(args[idx].clone());
                }
                other => {
                    return Err(anyhow!(
                        "unknown top-level argument '{}'. Use 'publish-world' for publish flags like --skip-ipns, or use run/--slug/--listen/--kubo-url/--owner/--log-level/--log-file for server mode.",
                        other
                    ));
                }
            }
            idx += 1;
        }
    }

    let default_world_slug = DEFAULT_WORLD_SLUG.to_string();
    let default_kubo_url = DEFAULT_KUBO_API_URL.to_string();

    let runtime_slug = if run_arg_mode {
        Some(
            world_slug_override
                .clone()
                .ok_or_else(|| anyhow!("--slug is required for server mode"))?,
        )
    } else {
        None
    };

    // Apply runtime values with precedence: CLI args > config file > env vars > defaults.
    if let Some(slug) = runtime_slug.as_deref() {
        let normalized_slug = normalize_world_key_name(slug);
        let runtime_config_path = runtime_config_path(&normalized_slug);
        let runtime_file_config = load_runtime_file_config(&runtime_config_path)?;

        if listen_addr == DEFAULT_LISTEN_ADDR {
            if let Some(cfg_listen) = runtime_file_config.listen.clone() {
                listen_addr = cfg_listen;
            } else if let Ok(env_listen) = std::env::var("MA_LISTEN") {
                listen_addr = env_listen;
            }
        }

        if kubo_url_override.is_none() {
            if let Some(cfg_kubo) = runtime_file_config.kubo_api_url.clone() {
                kubo_url_override = Some(cfg_kubo);
            } else if let Ok(env_kubo) = std::env::var("MA_KUBO_API_URL") {
                kubo_url_override = Some(env_kubo);
            }
        }

        if owner_override.is_none() {
            if let Some(cfg_owner) = runtime_file_config.owner.clone() {
                owner_override = Some(cfg_owner);
            } else if let Ok(env_owner) = std::env::var("MA_WORLD_OWNER") {
                owner_override = Some(env_owner);
            }
        }

        if log_level == "info" {
            if let Some(cfg_level) = runtime_file_config.log_level.clone() {
                log_level = cfg_level;
            } else if let Ok(env_level) = std::env::var("MA_LOG_LEVEL") {
                log_level = env_level;
            }
        }

        if log_file.is_none() {
            if let Some(cfg_file) = runtime_file_config.log_file.clone() {
                log_file = Some(expand_tilde_path(&cfg_file));
            } else if let Ok(env_file) = std::env::var("MA_LOG_FILE") {
                log_file = Some(expand_tilde_path(&env_file));
            } else {
                log_file = Some(
                    xdg_data_home()
                        .join("ma")
                        .join("worlds")
                        .join(&normalized_slug)
                        .join("ma-world.log"),
                );
            }
        }
    }

    if args.len() >= 2 && args[1] == "check-kubo-ipns" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;
        let mut key_override: Option<String> = None;
        let mut ipns_timeout_ms: u64 = 15_000;
        let mut ipns_retries: u32 = 3;
        let mut ipns_backoff_ms: u64 = 1_000;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                "--key" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --key"));
                    }
                    key_override = Some(args[idx].clone());
                }
                "--ipns-timeout-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-timeout-ms"));
                    }
                    ipns_timeout_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-timeout-ms: {}", e))?;
                }
                "--ipns-retries" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-retries"));
                    }
                    ipns_retries = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-retries: {}", e))?;
                }
                "--ipns-backoff-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-backoff-ms"));
                    }
                    ipns_backoff_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-backoff-ms: {}", e))?;
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for check-kubo-ipns (supported: --slug, --world-dir, --key, --ipns-timeout-ms, --ipns-retries, --ipns-backoff-ms)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let loaded = load_world_authoring(&world_dir)?;
        let kubo_url = loaded.config.kubo.api_url.clone();
        let key_name = key_override.unwrap_or_else(|| loaded.config.kubo.world_ipns_key_name.clone());

        let available = list_kubo_key_names(&kubo_url).await?;
        if !available.iter().any(|candidate| candidate == &key_name) {
            return Err(anyhow!(
                "Kubo key '{}' is missing (run ensure-kubo-keys first)",
                key_name
            ));
        }

        let probe = serde_json::json!({
            "kind": "ma.kubo-healthcheck.v1",
            "timestamp": Utc::now().to_rfc3339(),
            "world": loaded.world_manifest.world_id,
        });
        let probe_cid = dag_put_dag_cbor(&kubo_url, &probe).await?;

        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_millis(ipns_timeout_ms),
            ..IpnsPublishOptions::default()
        };
        let published = ipns_publish_with_retry(
            &kubo_url,
            &key_name,
            &probe_cid,
            &ipns_options,
            ipns_retries,
            Duration::from_millis(ipns_backoff_ms),
        )
        .await?;

        println!("check-kubo-ipns OK");
        println!("  key: {}", key_name);
        println!("  probe_cid: {}", probe_cid);
        println!("  published: {}", published);
        println!("  retries: {}", ipns_retries);
        println!("  timeout_ms: {}", ipns_timeout_ms);
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "init-world" {
        return Err(anyhow!("init-world has been removed"));
    }

    if args.len() >= 2 && args[1] == "ensure-kubo-keys" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for ensure-kubo-keys (supported: --slug, --world-dir)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let loaded = load_world_authoring(&world_dir)?;
        let kubo_url = loaded.config.kubo.api_url.clone();
        let mut available = list_kubo_key_names(&kubo_url).await?;

        let mut expected = loaded.config.kubo.required_keys.clone();
        expected.push(loaded.config.kubo.world_ipns_key_name.clone());
        for actor in loaded.actors.values() {
            if actor.actor_type != "room" {
                continue;
            }
            if let Some(fragment) = did_fragment(&actor.id) {
                expected.push(fragment.to_string());
            }
        }
        expected.sort();
        expected.dedup();

        let mut created = Vec::new();
        for key in &expected {
            if available.iter().any(|candidate| candidate == key) {
                continue;
            }
            generate_kubo_key(&kubo_url, key).await?;
            created.push(key.clone());
            available.push(key.clone());
        }

        println!("ensure-kubo-keys OK");
        if created.is_empty() {
            println!("  created: (none)");
        } else {
            println!("  created: {}", created.join(", "));
        }
        println!("  required: {}", expected.join(", "));
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "check-kubo-keys" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for check-kubo-keys (supported: --slug, --world-dir)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let loaded = load_world_authoring(&world_dir)?;
        let available = list_kubo_key_names(&loaded.config.kubo.api_url).await?;

        let mut expected = loaded.config.kubo.required_keys.clone();
        expected.push(loaded.config.kubo.world_ipns_key_name.clone());
        for actor in loaded.actors.values() {
            if actor.actor_type != "room" {
                continue;
            }
            if let Some(fragment) = did_fragment(&actor.id) {
                expected.push(fragment.to_string());
            }
        }
        expected.sort();
        expected.dedup();

        let missing = expected
            .iter()
            .filter(|key| !available.iter().any(|candidate| candidate == *key))
            .cloned()
            .collect::<Vec<_>>();

        if !missing.is_empty() {
            return Err(anyhow!(
                "missing Kubo key(s): {}",
                missing.join(", ")
            ));
        }

        println!("kubo key check OK");
        println!("  required: {}", expected.join(", "));
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "publish-world" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;
        let mut skip_ipns = false;
        let mut allow_partial_ipns = false;
        let mut ipns_timeout_ms: u64 = 15_000;
        let mut ipns_retries: u32 = 3;
        let mut ipns_backoff_ms: u64 = 1_000;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                "--skip-ipns" => {
                    skip_ipns = true;
                }
                "--allow-partial-ipns" => {
                    allow_partial_ipns = true;
                }
                "--ipns-timeout-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-timeout-ms"));
                    }
                    ipns_timeout_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-timeout-ms: {}", e))?;
                }
                "--ipns-retries" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-retries"));
                    }
                    ipns_retries = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-retries: {}", e))?;
                }
                "--ipns-backoff-ms" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --ipns-backoff-ms"));
                    }
                    ipns_backoff_ms = args[idx]
                        .parse()
                        .map_err(|e| anyhow!("invalid --ipns-backoff-ms: {}", e))?;
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for publish-world (supported: --slug, --world-dir, --skip-ipns, --allow-partial-ipns, --ipns-timeout-ms, --ipns-retries, --ipns-backoff-ms)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let mut loaded = load_world_authoring(&world_dir)?;
        let kubo_url = loaded.config.kubo.api_url.clone();

        let mut actor_cids: HashMap<String, String> = HashMap::new();
        let actor_ids = loaded.actors.keys().cloned().collect::<Vec<_>>();
        for actor_id in actor_ids {
            let actor = loaded
                .actors
                .get(&actor_id)
                .ok_or_else(|| anyhow!("missing actor payload for {}", actor_id))?;
            let cid = dag_put_dag_cbor(&kubo_url, actor).await?;
            actor_cids.insert(actor_id, cid);
        }

        for (actor_id, item) in &mut loaded.actor_registry.actors {
            if let Some(cid) = actor_cids.get(actor_id) {
                item.cid = cid.clone();
            }
        }

        let actor_registry_cid = dag_put_dag_cbor(&kubo_url, &loaded.actor_registry).await?;
        loaded.world_root.refs.actor_registry_cid = actor_registry_cid.clone();
        let world_root_cid = dag_put_dag_cbor(&kubo_url, &loaded.world_root).await?;

        loaded.world_manifest.world_root_cid = world_root_cid.clone();
        loaded.world_manifest.refs.actor_registry_cid = actor_registry_cid.clone();
        let world_manifest_cid = dag_put_dag_cbor(&kubo_url, &loaded.world_manifest).await?;

        let mut ipns_failures: Vec<String> = Vec::new();
        let ipns_options = IpnsPublishOptions {
            timeout: Duration::from_millis(ipns_timeout_ms),
            ..IpnsPublishOptions::default()
        };

        if loaded.config.publish.publish_world_ipns && !skip_ipns {
            let available = list_kubo_key_names(&kubo_url).await?;
            let mut required = loaded.config.kubo.required_keys.clone();
            required.push(loaded.config.kubo.world_ipns_key_name.clone());
            for actor in loaded.actors.values() {
                if actor.actor_type != "room" {
                    continue;
                }
                if let Some(fragment) = did_fragment(&actor.id) {
                    required.push(fragment.to_string());
                }
            }
            required.sort();
            required.dedup();
            let missing = required
                .iter()
                .filter(|key| !available.iter().any(|candidate| candidate == *key))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                let missing_msg = format!("cannot publish IPNS: missing Kubo key(s): {}", missing.join(", "));
                if allow_partial_ipns {
                    ipns_failures.push(missing_msg);
                } else {
                    return Err(anyhow!(missing_msg));
                }
            }

            if ipns_failures.is_empty() {
                match ipns_publish_with_retry(
                    &kubo_url,
                    &loaded.config.kubo.world_ipns_key_name,
                    &world_manifest_cid,
                    &ipns_options,
                    ipns_retries,
                    Duration::from_millis(ipns_backoff_ms),
                )
                .await
                {
                    Ok(published) => println!("world manifest published to IPNS: {}", published),
                    Err(err) => {
                        let msg = format!(
                            "world manifest IPNS publish failed (key={}): {}",
                            loaded.config.kubo.world_ipns_key_name,
                            err
                        );
                        if allow_partial_ipns {
                            ipns_failures.push(msg);
                        } else {
                            return Err(anyhow!(msg));
                        }
                    }
                }

                for actor in loaded.actors.values() {
                    if actor.actor_type != "room" {
                        continue;
                    }
                    let room_key = did_fragment(&actor.id)
                        .ok_or_else(|| anyhow!("room actor id missing fragment: {}", actor.id))?;
                    let room_cid = actor_cids
                        .get(&actor.id)
                        .ok_or_else(|| anyhow!("missing actor cid for room {}", actor.id))?;
                    match ipns_publish_with_retry(
                        &kubo_url,
                        room_key,
                        room_cid,
                        &ipns_options,
                        ipns_retries,
                        Duration::from_millis(ipns_backoff_ms),
                    )
                    .await
                    {
                        Ok(room_published) => {
                            println!("room actor published to IPNS ({}): {}", room_key, room_published)
                        }
                        Err(err) => {
                            let msg = format!(
                                "room actor IPNS publish failed (key={}): {}",
                                room_key, err
                            );
                            if allow_partial_ipns {
                                ipns_failures.push(msg);
                            } else {
                                return Err(anyhow!(msg));
                            }
                        }
                    }
                }
            }
        } else {
            println!("skipped IPNS publish (publish_world_ipns=false or --skip-ipns)");
        }

        if ipns_failures.is_empty() {
            println!("publish-world OK");
        } else {
            println!("publish-world PARTIAL (IPFS/IPLD succeeded; IPNS had issues)");
            for failure in &ipns_failures {
                println!("  ipns_issue: {}", failure);
            }
        }
        println!("  world_dir: {}", loaded.world_dir.display());
        println!("  actor_registry_cid: {}", actor_registry_cid);
        println!("  world_root_cid: {}", world_root_cid);
        println!("  world_manifest_cid: {}", world_manifest_cid);
        if loaded.config.publish.publish_world_ipns && !skip_ipns {
            println!("  ipns_retries: {}", ipns_retries);
            println!("  ipns_timeout_ms: {}", ipns_timeout_ms);
            println!("  ipns_backoff_ms: {}", ipns_backoff_ms);
            println!("  allow_partial_ipns: {}", allow_partial_ipns);
        }
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "validate-world" {
        let mut world_slug = default_world_slug.clone();
        let mut world_dir_override: Option<PathBuf> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--world-dir" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --world-dir"));
                    }
                    world_dir_override = Some(PathBuf::from(&args[idx]));
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for validate-world (supported: --slug, --world-dir)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let world_dir = world_dir_override.unwrap_or_else(|| default_world_dir(&world_slug));
        let report = validate_world_authoring(&world_dir)?;
        println!("world schema validation OK");
        println!("  world_dir: {}", report.world_dir);
        println!("  config: {}", report.config_path);
        println!("  authoring: {}", report.authoring_dir);
        println!("  actors: {}", report.actor_count);
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "verify-root" {
        let mut world_slug = default_world_slug.clone();
        let mut root_cid: Option<String> = None;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--root-cid" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --root-cid"));
                    }
                    root_cid = Some(args[idx].clone());
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for verify-root (supported: --slug, --root-cid)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let root_cid = root_cid.ok_or_else(|| anyhow!("--root-cid is required for verify-root"))?;
        let normalized_slug = normalize_world_key_name(&world_slug);
        let runtime_cfg_path = runtime_config_path(&normalized_slug);
        let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
        let kubo_url = runtime_cfg
            .kubo_api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_KUBO_API_URL.to_string());
        let iroh_path = runtime_cfg
            .iroh_secret
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));
        let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
            .ok_or_else(|| anyhow!(
                "missing iroh secret at {}. Create it with: ma-world --gen-iroh-secret --slug {}",
                iroh_path.display(),
                normalized_slug
            ))?;
        let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);
        let world_key_name = normalize_world_key_name(&normalized_slug);
        let did_identifier = ensure_kubo_key_id(&kubo_url, &world_key_name).await?;
        let world_did = Did::new(&did_identifier, &normalized_slug)
            .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;

        let world = World::new(
            EntryAcl {
                allow_all: true,
                allow_owner: true,
                allowed_dids: HashSet::new(),
                source: "*".to_string(),
            },
            kubo_url.clone(),
            normalized_slug.clone(),
        );
        world.set_world_master_key(world_master_key).await;
        world.set_world_did(&world_did.id()).await?;

        let rooms_loaded = world.load_from_world_cid(&root_cid).await?;
        let mut verified_state_cid = String::new();
        if let Some(state_cid) = world.state_cid().await {
            let _ = world.load_encrypted_state(&state_cid).await?;
            verified_state_cid = state_cid;
        }

        println!("verify-root OK");
        println!("  slug: {}", normalized_slug);
        println!("  root_cid: {}", root_cid);
        println!("  rooms_loaded: {}", rooms_loaded);
        if verified_state_cid.is_empty() {
            println!("  state_cid: (none)");
        } else {
            println!("  state_cid: {}", verified_state_cid);
        }
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "restore-root" {
        let mut world_slug = default_world_slug.clone();
        let mut root_cid: Option<String> = None;
        let mut dry_run = false;

        let mut idx = 2usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--slug" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --slug"));
                    }
                    world_slug = args[idx].clone();
                }
                "--root-cid" => {
                    idx += 1;
                    if idx >= args.len() {
                        return Err(anyhow!("missing value for --root-cid"));
                    }
                    root_cid = Some(args[idx].clone());
                }
                "--dry-run" => {
                    dry_run = true;
                }
                other => {
                    return Err(anyhow!(
                        "unknown argument '{}' for restore-root (supported: --slug, --root-cid, --dry-run)",
                        other
                    ));
                }
            }
            idx += 1;
        }

        let root_cid = root_cid.ok_or_else(|| anyhow!("--root-cid is required for restore-root"))?;
        let normalized_slug = normalize_world_key_name(&world_slug);
        let runtime_cfg_path = runtime_config_path(&normalized_slug);
        let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
        let kubo_url = runtime_cfg
            .kubo_api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_KUBO_API_URL.to_string());
        let iroh_path = runtime_cfg
            .iroh_secret
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| runtime_iroh_secret_default_path(&normalized_slug));
        let secret_key = load_persisted_iroh_secret_key(&iroh_path)?
            .ok_or_else(|| anyhow!(
                "missing iroh secret at {}. Create it with: ma-world --gen-iroh-secret --slug {}",
                iroh_path.display(),
                normalized_slug
            ))?;
        let world_master_key = derive_world_master_key(&secret_key, &normalized_slug);
        let world_key_name = normalize_world_key_name(&normalized_slug);
        let did_identifier = ensure_kubo_key_id(&kubo_url, &world_key_name).await?;
        let world_did = Did::new(&did_identifier, &normalized_slug)
            .map_err(|e| anyhow!("failed to build world DID from key id '{}': {}", did_identifier, e))?;

        let world = World::new(
            EntryAcl {
                allow_all: true,
                allow_owner: true,
                allowed_dids: HashSet::new(),
                source: "*".to_string(),
            },
            kubo_url.clone(),
            normalized_slug.clone(),
        );
        world.set_world_master_key(world_master_key).await;
        world.set_world_did(&world_did.id()).await?;

        let rooms_loaded = world.load_from_world_cid(&root_cid).await?;
        if let Some(state_cid) = world.state_cid().await {
            let _ = world.load_encrypted_state(&state_cid).await?;
        }

        if dry_run {
            println!("restore-root DRY-RUN OK");
            println!("  slug: {}", normalized_slug);
            println!("  input_root_cid: {}", root_cid);
            println!("  rooms_loaded: {}", rooms_loaded);
            return Ok(());
        }

        let (new_state_cid, new_root_cid) = world.save_and_publish().await?;

        println!("restore-root OK");
        println!("  slug: {}", normalized_slug);
        println!("  input_root_cid: {}", root_cid);
        println!("  rooms_loaded: {}", rooms_loaded);
        println!("  output_state_cid: {}", new_state_cid);
        println!("  output_root_cid: {}", new_root_cid);
        return Ok(());
    }

    if args.len() >= 2 && !run_arg_mode {
        return Err(anyhow!(
            "unknown command '{}'. Use --help to list available commands.",
            args[1]
        ));
    }

    // Initialize logging with configurable level and optional file output.
    // Default filters keep normal runs readable while allowing richer transport traces in debug mode.
    let normalized_level = log_level.trim().to_lowercase();
    let iroh_level = if normalized_level == "debug" || normalized_level == "trace" {
        normalized_level.as_str()
    } else {
        "info"
    };
    let directives = [
        format!("ma_world={}", normalized_level),
        format!("ma_core={}", normalized_level),
        format!("iroh={}", iroh_level),
        format!("iroh_net={}", iroh_level),
        format!("iroh_relay={}", iroh_level),
    ];
    let mut env_filter = tracing_subscriber::EnvFilter::from_default_env();
    for directive in directives {
        env_filter = env_filter.add_directive(directive.parse()?);
    }

    if let Some(log_file_path) = &log_file {
        // Create parent directory if it doesn't exist
        if let Some(parent) = log_file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)?;

        let stdout_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(true)
            .with_writer(std::io::stdout);

        let file_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(false)
            .with_writer(file);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stdout_layer)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(true)
            .init();
    }

    info!("Starting ma-world server");
    info!("Log level: {}", log_level);
    if let Some(log_path) = &log_file {
        info!("Logging to file: {}", log_path.display());
    }

    let runtime_slug = runtime_slug
        .ok_or_else(|| anyhow!("--slug is required for server mode"))?;
    let world_slug = normalize_world_key_name(&runtime_slug);
    let runtime_cfg_path = runtime_config_path(&world_slug);
    let runtime_cfg = load_runtime_file_config(&runtime_cfg_path)?;
    let authored_world_dir = default_world_dir(&world_slug);
    let authored_world = load_world_authoring(&authored_world_dir).ok();
    let authored_global_acl_cid = authored_world
        .as_ref()
        .and_then(|loaded| loaded.world_root.refs.global_acl_cid.clone())
        .filter(|cid| !cid.trim().is_empty());

    let kubo_url = kubo_url_override
        .or_else(|| runtime_cfg.kubo_api_url.clone())
        .or_else(|| std::env::var("MA_KUBO_API_URL").ok())
        .unwrap_or_else(|| default_kubo_url.clone());
    let status_api_enabled = runtime_cfg.status_api_enabled.unwrap_or(true);
    let admin_api_enabled = runtime_cfg.admin_api_enabled.unwrap_or(false);
    let admin_api_password = if admin_api_enabled {
        Some(
            std::env::var("MA_WORLD_ADMIN_API_PASSWORD")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    runtime_cfg
                        .admin_api_password
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                })
                .ok_or_else(|| {
                    anyhow!(
                        "missing admin API password: set admin_api_password in {} or MA_WORLD_ADMIN_API_PASSWORD",
                        runtime_cfg_path.display()
                    )
                })?,
        )
    } else {
        None
    };
    let entry_acl = load_entry_acl()?;
    let world = Arc::new(World::new(
        entry_acl,
        kubo_url.clone(),
        world_slug.clone(),
    ));
    let configured_lang_cid = runtime_cfg
        .lang_cid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let default_lang_cid = compiled_default_lang_cid();
    let startup_lang_cid = configured_lang_cid
        .clone()
        .or(default_lang_cid.clone());
    if let Some(lang_cid) = startup_lang_cid {
        world.set_lang_cid(Some(lang_cid)).await;
    }
    info!("Runtime world slug: {}", world_slug);
    info!("Runtime config path: {}", runtime_cfg_path.display());
    info!("Status API enabled: {}", status_api_enabled);
    info!("Admin API enabled: {}", admin_api_enabled);

    if let Some(owner) = owner_override.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        world.set_owner_did(owner).await?;
    }

    world.create_room(DEFAULT_ROOM.to_string()).await?;

    // Best-effort local auto-unlock for single-user dev/test flow.
    // If authoring + key material are present, start in unlocked mode without status-page interaction.
    if let Ok(loaded_for_unlock) = load_world_authoring(&authored_world_dir) {
        let master_key_rel = PathBuf::from(loaded_for_unlock.config.crypto.world_master_key_file.clone());
        let master_key_path = if master_key_rel.is_absolute() {
            master_key_rel
        } else {
            loaded_for_unlock.world_dir.join(master_key_rel)
        };

        world
            .set_unlock_context(loaded_for_unlock.world_dir.clone(), master_key_path.clone())
            .await;

        match fs::read(&master_key_path) {
            Ok(bytes) => {
                let master_key: Result<[u8; 32], _> = bytes.as_slice().try_into();
                match master_key {
                    Ok(key) => {
                        world.set_world_master_key(key).await;
                        match unlock_actor_secret_bundles(&loaded_for_unlock) {
                            Ok(bundles) => {
                                let count = bundles.len();
                                if let Err(err) = world.install_actor_secrets(&bundles).await {
                                    warn!("auto-unlock: failed installing actor secrets: {}", err);
                                } else {
                                    *world.unlocked.write().await = true;
                                    info!("auto-unlock: enabled with {} actor bundles", count);
                                }
                            }
                            Err(err) => {
                                warn!("auto-unlock: failed unlocking actor secret bundles: {}", err);
                            }
                        }
                    }
                    Err(_) => {
                        warn!(
                            "auto-unlock: world master key must be 32 bytes in {}",
                            master_key_path.display()
                        );
                    }
                }
            }
            Err(err) => {
                warn!(
                    "auto-unlock: could not read world master key {}: {}",
                    master_key_path.display(),
                    err
                );
            }
        }
    }

    // Passphrase-based auto-unlock from runtime config.
    if !world.is_unlocked().await {
        if let Some(passphrase) = runtime_cfg.unlock_passphrase.as_deref().filter(|s| !s.trim().is_empty()) {
            let bundle_file = runtime_cfg
                .unlock_bundle_file
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    runtime_config_path(&world_slug)
                        .with_file_name(format!("{}_bundle.json", world_slug))
                });
            match fs::read_to_string(&bundle_file) {
                Ok(bundle_json) => {
                    match world.unlock_runtime(passphrase, &bundle_json).await {
                        Ok(count) => {
                            info!(
                                "auto-unlock (passphrase): enabled with {} actor bundles from {}",
                                count,
                                bundle_file.display()
                            );
                        }
                        Err(err) => {
                            warn!(
                                "auto-unlock (passphrase): unlock failed for bundle {}: {}",
                                bundle_file.display(),
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "auto-unlock (passphrase): could not read bundle {}: {}",
                        bundle_file.display(),
                        err
                    );
                }
            }
        }
    }

    // Bind status listener before iroh endpoint setup so listen failures abort early.
    let listener = if status_api_enabled {
        Some(bind_status_listener(&listen_addr).await?)
    } else {
        None
    };
    let status_url = listener
        .as_ref()
        .map(|socket| socket.local_addr())
        .transpose()?
        .map(|addr| format!("http://{}/status.json", addr))
        .unwrap_or_else(|| "disabled".to_string());

    let key_path = runtime_cfg
        .iroh_secret
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_iroh_secret_default_path(&world_slug));
    let Some(secret_key) = load_persisted_iroh_secret_key(&key_path)? else {
        return Err(anyhow!(
            "missing iroh secret at {}. Create it explicitly with: ma-world --gen-iroh-secret {}",
            key_path.display(),
            key_path.display()
        ));
    };
    info!("Loaded persistent iroh identity from {}", key_path.display());
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .bind()
        .await?;

    // Ensure Kubo API is online before DID/IPNS bootstrap.
    wait_for_kubo_api(&kubo_url, 8).await?;

    if let Some(global_acl_cid) = authored_global_acl_cid.as_deref() {
        match world.load_global_capability_acl_from_cid(global_acl_cid).await {
            Ok(()) => info!("Loaded global capability ACL from CID {}", global_acl_cid),
            Err(err) => warn!(
                "Failed loading global capability ACL from CID {}: {}",
                global_acl_cid,
                err
            ),
        }
    }

    let run_result: Result<()> = async {
        let world_master_key = derive_world_master_key(endpoint.secret_key(), &world_slug);
        world.set_world_master_key(world_master_key).await;
        info!("World master key source: derived from iroh identity and world slug");

        let endpoint_id = endpoint.id().to_string();
        let world_did = ensure_world_did_document(
            &kubo_url,
            &world_slug,
            &endpoint_id,
            world_master_key,
        )
        .await?;
        world.set_world_did(&world_did).await?;
        info!("Runtime world DID: {}", world_did);

        let restore_root = match resolve_world_root_cid_from_did(&kubo_url, &world_did).await {
            Ok(r) => r,
            Err(err) => {
                warn!("Failed resolving world root CID from DID {}: {} — starting fresh", world_did, err);
                None
            }
        };
        if let Some(root_cid) = restore_root {
            match world.load_from_world_cid(&root_cid).await {
                Ok(rooms_loaded) => info!(
                    "Restored world from DID ma runtime fields: root_cid={} rooms={}",
                    root_cid, rooms_loaded
                ),
                Err(err) => warn!(
                    "Failed restoring world from DID ma runtime fields {}: {}",
                    root_cid, err
                ),
            }

            if let Some(state_cid) = world.state_cid().await {
                match world.load_encrypted_state(&state_cid).await {
                    Ok(new_root_cid) => info!(
                        "Restored encrypted runtime state: state_cid={} root_cid={}",
                        state_cid, new_root_cid
                    ),
                    Err(err) => warn!(
                        "Failed restoring encrypted runtime state {}: {}",
                        state_cid, err
                    ),
                }
            }
        }

        if let Some(lang_cid) = configured_lang_cid.clone() {
            world.set_lang_cid(Some(lang_cid.clone())).await;
            info!("Applied runtime config lang_cid override: {}", lang_cid);
        }

        if world.world_cid().await.is_none() {
            let (state_cid, root_cid) = world.save_encrypted_state().await?;
            info!(
                "Bootstrapped world state with lobby snapshot: state_cid={} root_cid={} — IPNS deferred to background publisher",
                state_cid,
                root_cid
            );
        }

        // Publish runtime ma links promptly on startup so DID consumers can IPLD-traverse
        // via ma.world/ma.state_cid without waiting for the periodic publish timer.
        {
            let initial_state_cid = world.state_cid().await;
            let initial_root_cid = world.world_cid().await;
            if initial_state_cid.is_some() && initial_root_cid.is_some() {
                let world_for_initial_ipns = world.clone();
                tokio::spawn(async move {
                    info!("Startup IPNS publish: refreshing DID ma runtime links");
                    match world_for_initial_ipns.publish_to_ipns().await {
                        Ok(()) => info!("Startup IPNS publish: completed"),
                        Err(err) => warn!("Startup IPNS publish failed: {}", err),
                    }
                });
            }
        }

        {
            let world_for_washer = world.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(OBJECT_WASHER_INTERVAL_SECS));
                loop {
                    ticker.tick().await;
                    match world_for_washer.flush_dirty_object_blobs().await {
                        Ok(count) if count > 0 => {
                            debug!("object washer flushed {} dirty blobs", count);
                        }
                        Ok(_) => {}
                        Err(err) => {
                            warn!("object washer flush failed: {}", err);
                        }
                    }
                }
            });
        }

        // Background IPNS publisher: periodically publishes to IPNS if dirty.
        {
            let ipns_interval = std::env::var("MA_WORLD_IPNS_PUBLISH_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(IPNS_PUBLISH_INTERVAL_SECS_DEFAULT);
            let world_for_ipns = world.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(ipns_interval));
                ticker.tick().await; // skip immediate first tick
                loop {
                    ticker.tick().await;
                    if !world_for_ipns.is_ipns_dirty().await {
                        continue;
                    }
                    info!("IPNS publish timer: dirty state detected, publishing...");
                    match world_for_ipns.publish_to_ipns().await {
                        Ok(()) => info!("IPNS publish timer: published successfully"),
                        Err(err) => warn!("IPNS publish timer: failed: {}", err),
                    }
                }
            });
        }

        let did_cache = Arc::new(RwLock::new(HashMap::new()));
        let ipfs_protocol = IpfsProtocol {
            kubo_url: kubo_url.clone(),
            did_cache: did_cache.clone(),
        };
        let online_started = Instant::now();
        let online_status = match tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await {
            Ok(_) => format!("ok in {}ms", online_started.elapsed().as_millis()),
            Err(_) => format!("timeout after {}ms", online_started.elapsed().as_millis()),
        };
        let endpoint_addr = endpoint.addr();

        let direct_addresses = endpoint_addr
            .ip_addrs()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>();
        let multiaddrs = endpoint_addr
            .ip_addrs()
            .map(socket_addr_to_multiaddr)
            .collect::<Vec<_>>();
        let relay_urls = endpoint_addr
            .relay_urls()
            .map(|url| url.to_string())
            .collect::<Vec<_>>();

        let world_info = WorldInfo {
            name: world_slug.clone(),
            world_did: world_did.clone(),
            status_url: status_url.clone(),
            endpoint_id: endpoint_id.clone(),
            direct_addresses,
            multiaddrs,
            relay_urls,
            kubo_url: kubo_url.clone(),
            location_hint: format!("/iroh/{endpoint_id}"),
            entry_acl: world.entry_acl_source().await,
            started_at: Utc::now().to_rfc3339(),
            capabilities: vec![
                LaneCapability::for_lane(WorldLane::Inbox),
                LaneCapability::for_lane(WorldLane::Avatar),
            ],
        };

        if let Some(listener) = listener {
            let status_world = world.clone();
            let status_info = world_info.clone();
            let status_www_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("www");
            let admin_api_password = admin_api_password.clone();
            tokio::spawn(async move {
                if let Err(err) = status::serve(
                    listener,
                    status_world,
                    status_info,
                    status_www_root,
                    admin_api_enabled,
                    admin_api_password,
                )
                .await
                {
                    error!("status server failed: {}", err);
                }
            });
        }

        info!("Created default room: {}", DEFAULT_ROOM);
        info!("World endpoint id: {}", world_info.endpoint_id);
        info!("World status page: {}", world_info.status_url);
        info!("Inbox protocol ALPN: {}", String::from_utf8_lossy(INBOX_ALPN));
        info!("Avatar protocol ALPN: {}", String::from_utf8_lossy(AVATAR_ALPN));
        info!("IPFS protocol ALPN (citizenship): {}", String::from_utf8_lossy(IPFS_ALPN));
        info!("Presence protocol ALPN (outbound push to agents): {}", String::from_utf8_lossy(PRESENCE_ALPN));
        info!("World entry ACL: {}", world_info.entry_acl);
        info!("Optional DID field ma:presenceHint = {}", world_info.location_hint);
        info!("Iroh online readiness: {}", online_status);

        for relay_url in &world_info.relay_urls {
            let probe = probe_relay(relay_url).await;
            info!("Relay probe {} -> {}", relay_url, probe);
        }

        println!("\n╔══════════════════════════════════════════════════════════╗");
        println!("║ ma-world Server                                         ║");
        println!("║ status page:   {:<41} ║", trim_console(&world_info.status_url, 41));
        println!("║ kubo API:      {:<41} ║", trim_console(&world_info.kubo_url, 41));
        println!("╚══════════════════════════════════════════════════════════╝");
        println!("world endpoint full: {}\n", world_info.endpoint_id);

        world
            .record_event(format!("world online at {}", world_info.status_url))
            .await;

        // Wait for unlock before accepting protocol connections.
        // While locked, a gate router responds with "world is locked" on all ALPNs.
        if !world.is_unlocked().await {
            let gate = LockedGateProtocol;
            let gate_router = Router::builder(endpoint.clone())
                .accept(INBOX_ALPN, gate.clone())
                .accept(AVATAR_ALPN, gate.clone())
                .accept(IPFS_ALPN, gate)
                .spawn();
            world
                .record_event("world runtime locked; gate router active — waiting for unlock".to_string())
                .await;
            info!("World locked — gate router active, waiting for unlock via status page at {}", world_info.status_url);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if world.is_unlocked().await {
                    break;
                }
            }
            gate_router.shutdown().await?;
            world
                .record_event("world unlocked — starting protocol lanes".to_string())
                .await;
            info!("World unlocked — starting protocol lanes");
        }

        let inbox_protocol = WorldProtocol {
            world: world.clone(),
            endpoint: endpoint.clone(),
            endpoint_id: endpoint_id.clone(),
            did_cache: did_cache.clone(),
            push_stream_cache: Arc::new(Mutex::new(HashMap::new())),
            push_timeout_cooldown: Arc::new(Mutex::new(HashMap::new())),
            lane: WorldLane::Inbox,
        };
        let avatar_protocol = WorldProtocol {
            world: world.clone(),
            endpoint: endpoint.clone(),
            endpoint_id: endpoint_id.clone(),
            did_cache: did_cache.clone(),
            push_stream_cache: inbox_protocol.push_stream_cache.clone(),
            push_timeout_cooldown: inbox_protocol.push_timeout_cooldown.clone(),
            lane: WorldLane::Avatar,
        };
        let router = Router::builder(endpoint.clone())
            .accept(INBOX_ALPN, inbox_protocol.clone())
            .accept(AVATAR_ALPN, avatar_protocol)
            .accept(IPFS_ALPN, ipfs_protocol)
            .spawn();

        // Join the ma broadcast channel.
        let broadcast_result = join_broadcast_channel(endpoint.clone()).await;

        let presence_probe_secs = env::var("MA_PRESENCE_PROBE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(PRESENCE_PROBE_INTERVAL_SECS_DEFAULT);
        let presence_stale_secs = env::var("MA_PRESENCE_STALE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 2)
            .unwrap_or(PRESENCE_STALE_AFTER_SECS_DEFAULT)
            .max(presence_probe_secs + 1);

        world
            .configure_room_avatar_ttl(Duration::from_secs(presence_stale_secs))
            .await;

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Announce world startup on the broadcast channel.
        match broadcast_result {
            Ok((_gossip, sender)) => {
                let startup_msg = format!(
                    r#"{{"kind":"world.online","world":"{}","ts":"{}"}}"#,
                    world_did,
                    Utc::now().to_rfc3339()
                );
                if let Err(err) = gossip_send_text(&sender, &startup_msg).await {
                    warn!("Broadcast startup announce failed: {}", err);
                } else {
                    info!("Broadcast: world online announced on {}", BROADCAST_TOPIC);
                }
                // Keep sender alive in a background task for the shutdown announce.
                let mut gossip_shutdown = shutdown_rx.clone();
                let shutdown_world_did = world_did.clone();
                tokio::spawn(async move {
                    let _ = gossip_shutdown.changed().await;
                    let shutdown_msg = format!(
                        r#"{{"kind":"world.offline","world":"{}","ts":"{}"}}"#,
                        shutdown_world_did,
                        Utc::now().to_rfc3339()
                    );
                    let _ = gossip_send_text(&sender, &shutdown_msg).await;
                });
            }
            Err(err) => {
                warn!("Broadcast gossip join failed: {}", err);
            }
        }

        let inbox_presence = inbox_protocol.clone();
        let mut probe_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(presence_probe_secs));
            let stale_after = Duration::from_secs(presence_stale_secs);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = probe_shutdown.changed() => { break; }
                }
                if *probe_shutdown.borrow() {
                    break;
                }
                let _ = inbox_presence.world.prune_stale_avatars(stale_after).await;
            }
        });

        let dispatch_protocol = inbox_protocol.clone();
        let mut dispatch_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(150));
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = dispatch_shutdown.changed() => { break; }
                }
                if *dispatch_shutdown.borrow() {
                    break;
                }
                dispatch_protocol.flush_pending_room_dispatches().await;
            }
        });

        let refresh_protocol = inbox_protocol.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let room_names = refresh_protocol.world.room_names().await;
            for room_name in room_names {
                let _ = refresh_protocol
                    .world
                    .enqueue_room_dispatch(&room_name, RoomDispatchTask::PresenceRefreshRequest)
                    .await;
            }
        });

        info!(
            "Presence pruning enabled: probe={}s stale_after={}s",
            presence_probe_secs,
            presence_stale_secs
        );

        world
            .record_event(format!("iroh online readiness: {}", online_status))
            .await;
        world
            .record_event(format!("world direct addrs: {}", world_info.direct_addresses.join(", ")))
            .await;
        world
            .record_event(format!("world relays: {}", world_info.relay_urls.join(", ")))
            .await;
        for relay_url in &world_info.relay_urls {
            let probe = probe_relay(relay_url).await;
            world
                .record_event(format!("relay probe {} -> {}", relay_url, probe))
                .await;
        }
        world
            .record_event(format!("entry acl: {}", world_info.entry_acl))
            .await;
        world
            .record_event(format!("optional published location hint: {}", world_info.location_hint))
            .await;
        world
            .record_event(format!("inbox protocol ready on ALPN {}", String::from_utf8_lossy(INBOX_ALPN)))
            .await;

        info!("World initialized. Waiting for connections...");
        let signal_name = wait_for_shutdown_signal().await?;
        info!("Received {} shutting down", signal_name);
        info!("Shutting down ma-world — stopping background tasks...");

        // Cancel the presence probe loop immediately so it stops trying to
        // reach stale/dead actor endpoints before we close the iroh router.
        let _ = shutdown_tx.send(true);

        info!("Shutting down ma-world — saving state...");
        match world.save_and_publish().await {
            Ok((state_cid, root_cid)) => {
                info!("State saved and published: state_cid={} root_cid={}", state_cid, root_cid);
            }
            Err(e) => {
                warn!("Failed to save state on shutdown: {}", e);
            }
        }

        // Give the iroh router a bounded window to drain open connections.
        // If it takes longer than 5 s we exit anyway — connections will time out
        // on the actor side.
        let shutdown_timeout = Duration::from_secs(5);
        match tokio::time::timeout(shutdown_timeout, router.shutdown()).await {
            Ok(Ok(())) => info!("Router shut down cleanly."),
            Ok(Err(e)) => warn!("Router shutdown error: {}", e),
            Err(_) => warn!("Router shutdown timed out after {}s; forcing exit.", shutdown_timeout.as_secs()),
        }

        Ok(())
    }
    .await;

    endpoint.close().await;
    run_result
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<&'static str> {
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok("SIGINT"),
        _ = sigterm.recv() => Ok("SIGTERM"),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<&'static str> {
    tokio::signal::ctrl_c().await?;
    Ok("SIGINT")
}

async fn bind_status_listener(listen_addr: &str) -> Result<TcpListener> {
    let socket: SocketAddr = listen_addr
        .parse()
        .map_err(|e| anyhow!("invalid --listen '{}': {}", listen_addr, e))?;
    TcpListener::bind(socket)
        .await
        .map_err(|e| anyhow!("failed to bind status listener on {}: {}", socket, e))
}



fn trim_console(input: &str, width: usize) -> String {
    let mut output = input.chars().take(width).collect::<String>();
    if output.len() < width {
        output.push_str(&" ".repeat(width - output.len()));
    }
    output
}

async fn probe_relay(relay_url: &str) -> String {
    let started = Instant::now();
    let client = match reqwest::Client::builder().timeout(RELAY_PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(err) => return format!("client-build-error: {}", err),
    };

    match client.get(relay_url).send().await {
        Ok(resp) => format!("http {} in {}ms", resp.status().as_u16(), started.elapsed().as_millis()),
        Err(err) => format!("error {} in {}ms", err, started.elapsed().as_millis()),
    }
}

fn load_entry_acl() -> Result<EntryAcl> {
    let raw = std::env::var(WORLD_ENTRY_ACL_ENV).unwrap_or_else(|_| DEFAULT_ENTRY_ACL.to_string());
    parse_entry_acl(&raw)
}

fn parse_entry_acl(raw: &str) -> Result<EntryAcl> {
    let tokens = raw
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return Err(anyhow!(
            "{} is empty; use '*' or comma separated did:ma:... values",
            WORLD_ENTRY_ACL_ENV
        ));
    }

    let mut allow_all = false;
    let mut allowed_dids = HashSet::new();
    let mut owner_token_present = false;
    for token in tokens {
        if token == "*" {
            allow_all = true;
            continue;
        }
        if token.eq_ignore_ascii_case("owner") {
            owner_token_present = true;
            continue;
        }

        let did = Did::try_from(token)?;
        allowed_dids.insert(did.id());
    }

    if !allow_all && allowed_dids.is_empty() && !owner_token_present {
        return Err(anyhow!(
            "{} must contain '*', 'owner', or at least one valid DID",
            WORLD_ENTRY_ACL_ENV
        ));
    }

    Ok(EntryAcl {
        allow_all,
        allow_owner: owner_token_present,
        allowed_dids,
        source: raw.trim().to_string(),
    })
}

