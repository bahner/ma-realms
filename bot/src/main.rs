use std::{env, fs, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use chrono::Utc;
use iroh::{endpoint::presets, Endpoint, EndpointAddr, EndpointId, RelayUrl};
use ma_core::{
    closet_command,
    closet_start,
    closet_submit_citizenship,
    default_ma_config_root,
    ensure_local_ipns_key_file,
    normalize_relay_url,
    DEFAULT_WORLD_RELAY_URL,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct StatusResponse {
    world: StatusWorld,
}

#[derive(Debug, Deserialize)]
struct StatusWorld {
    endpoint_id: String,
}

#[derive(Debug, Serialize)]
struct BotIdentityRecord {
    created_at: String,
    status_url: String,
    endpoint_id: String,
    session_id: String,
    did: String,
    fragment: String,
    key_name: Option<String>,
    ipns_private_key_path: String,
}

#[derive(Debug)]
struct Args {
    status_url: String,
    endpoint_id: Option<String>,
    name: Option<String>,
    description: Option<String>,
}

fn parse_args() -> Args {
    let mut status_url = String::from("http://127.0.0.1:5002");
    let mut endpoint_id: Option<String> = None;
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--status-url" => {
                if let Some(v) = iter.next() {
                    status_url = v;
                }
            }
            "--world-endpoint" => {
                endpoint_id = iter.next();
            }
            "--name" => {
                name = iter.next();
            }
            "--description" => {
                description = iter.next();
            }
            _ => {}
        }
    }

    Args {
        status_url,
        endpoint_id,
        name,
        description,
    }
}

async fn discover_endpoint_id(status_url: &str) -> Result<String> {
    let base = status_url.trim_end_matches('/');
    let url = format!("{base}/status.json");
    let status = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .context("failed to query status.json")?
        .error_for_status()
        .context("status endpoint returned non-success")?
        .json::<StatusResponse>()
        .await
        .context("failed to decode status.json")?;

    let endpoint = status.world.endpoint_id.trim().to_string();
    if endpoint.is_empty() {
        return Err(anyhow!("status.json contained empty world endpoint id"));
    }
    Ok(endpoint)
}

fn config_root() -> Result<PathBuf> {
    default_ma_config_root()
}

fn save_identity(record: &BotIdentityRecord) -> Result<PathBuf> {
    let root = config_root()?;
    fs::create_dir_all(&root).context("failed to create ~/.config/ma directory")?;

    let path = root.join(format!("{}.json", record.fragment));
    let content = serde_json::to_string_pretty(record)?;
    fs::write(&path, content).context("failed to write bot identity file")?;

    Ok(path)
}

fn ensure_local_ipns_key() -> Result<(Vec<u8>, PathBuf)> {
    let root = config_root()?;
    ensure_local_ipns_key_file(&root, "bot_ipns.key")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();

    let endpoint_id = match args.endpoint_id {
        Some(v) if !v.trim().is_empty() => v,
        _ => discover_endpoint_id(&args.status_url).await?,
    };

    let target: EndpointId = endpoint_id
        .trim()
        .parse()
        .map_err(|e| anyhow!("invalid endpoint id: {e}"))?;
    let relay_source = normalize_relay_url(DEFAULT_WORLD_RELAY_URL);
    let relay_url: RelayUrl = relay_source
        .parse()
        .map_err(|e| anyhow!("relay URL parse failed for '{}': {}", relay_source, e))?;
    let endpoint_addr = EndpointAddr::new(target).with_relay_url(relay_url);
    let (ipns_key_bytes, ipns_key_path) = ensure_local_ipns_key()?;

    let endpoint = Endpoint::builder(presets::N0)
        .bind()
        .await
        .map_err(|e| anyhow!("endpoint bind failed: {e}"))?;
    let _ = endpoint.online().await;

    println!("Using world endpoint: {endpoint_id}");
    println!("Bot endpoint: {}", endpoint.id());

    let start = closet_start(&endpoint, endpoint_addr.clone()).await?;
    if !start.ok {
        return Err(anyhow!("closet start failed: {}", start.message));
    }

    let session_id = start
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("closet start returned no session_id"))?;

    println!("Closet session started: {session_id}");

    if let Some(name) = args.name.as_ref() {
        let response = closet_command(
            &endpoint,
            endpoint_addr.clone(),
            session_id.clone(),
            format!("name {name}"),
        )
        .await?;
        if !response.ok {
            return Err(anyhow!("closet name failed: {}", response.message));
        }
    }

    if let Some(description) = args.description.as_ref() {
        let response = closet_command(
            &endpoint,
            endpoint_addr.clone(),
            session_id.clone(),
            format!("description {description}"),
        )
        .await?;
        if !response.ok {
            return Err(anyhow!("closet description failed: {}", response.message));
        }
    }

    let citizen = closet_submit_citizenship(
        &endpoint,
        endpoint_addr.clone(),
        session_id.clone(),
        base64::engine::general_purpose::STANDARD.encode(ipns_key_bytes),
    )
    .await?;

    if !citizen.ok {
        return Err(anyhow!("closet citizen failed: {}", citizen.message));
    }

    let did = citizen
        .did
        .clone()
        .ok_or_else(|| anyhow!("citizen response did not include did"))?;
    let fragment = citizen
        .fragment
        .clone()
        .ok_or_else(|| anyhow!("citizen response did not include fragment"))?;

    let record = BotIdentityRecord {
        created_at: Utc::now().to_rfc3339(),
        status_url: args.status_url,
        endpoint_id,
        session_id: session_id.clone(),
        did: did.clone(),
        fragment: fragment.clone(),
        key_name: citizen.key_name.clone(),
        ipns_private_key_path: ipns_key_path.display().to_string(),
    };

    let saved_path = save_identity(&record)?;

    let enter = closet_command(
        &endpoint,
        endpoint_addr,
        session_id.clone(),
        "enter",
    )
    .await?;

    println!("Citizen allocated DID: {did}");
    println!("Fragment: {fragment}");
    println!("Local IPNS key: {}", ipns_key_path.display());
    println!("Enter result: ok={} message='{}'", enter.ok, enter.message);
    println!("Saved bot identity metadata to: {}", saved_path.display());
    println!("Avatar enter attempted via closet lane.");

    endpoint.close().await;

    Ok(())
}
