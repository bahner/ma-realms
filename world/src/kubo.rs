use anyhow::{Result, anyhow};
use did_ma::{Did, Document};
use reqwest::multipart;
use serde::Serialize;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

pub async fn pin_add_named(kubo_url: &str, cid: &str, name: &str) -> Result<()> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/pin/add");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    client
        .post(url)
        .query(&[("arg", cid), ("recursive", "true"), ("name", name)])
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

pub async fn pin_rm(kubo_url: &str, cid: &str) -> Result<()> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/pin/rm");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    client
        .post(url)
        .query(&[("arg", cid), ("recursive", "true")])
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

pub async fn pin_update(kubo_url: &str, from_cid: &str, to_cid: &str) -> Result<()> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/pin/update");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    client
        .post(url)
        .query(&[
            ("arg", from_cid),
            ("arg", to_cid),
            ("unpin", "true"),
        ])
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

pub async fn fetch_did_document(kubo_url: &str, did: &Did) -> Result<Document> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/cat");
    let arg = format!("/ipns/{}", did.ipns);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()?;

    let body = client
        .post(url)
        .query(&[("arg", arg)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let document = Document::unmarshal(&body)
        .map_err(|err| anyhow!("failed to decode DID document from IPNS {}: {}", did.ipns, err))?;
    document.validate()?;
    document.verify()?;

    let expected = did.without_fragment().id();
    if document.id != expected {
        return Err(anyhow!(
            "DID document id mismatch for {}: got {}",
            expected,
            document.id
        ));
    }

    Ok(document)
}

#[derive(Debug, Deserialize)]
struct DagPutCid {
    #[serde(rename = "/")]
    slash: String,
}

#[derive(Debug, Deserialize)]
struct DagPutResponse {
    #[serde(default, rename = "Cid")]
    cid_upper: Option<DagPutCid>,
    #[serde(default)]
    cid: Option<DagPutCid>,
}

#[derive(Debug, Deserialize)]
struct NamePublishResponse {
    #[serde(default, rename = "Value")]
    value_upper: String,
    #[serde(default, rename = "value")]
    value_lower: String,
}

#[derive(Debug, Deserialize)]
struct NameResolveResponse {
    #[serde(default, rename = "Path")]
    path_upper: String,
    #[serde(default, rename = "path")]
    path_lower: String,
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    #[serde(default, rename = "Version")]
    version_upper: String,
    #[serde(default, rename = "version")]
    version_lower: String,
}

#[derive(Debug, Deserialize)]
struct KeyListEntry {
    #[serde(default, rename = "Name")]
    name: String,
    #[serde(default, rename = "name")]
    name_lower: String,
    #[serde(default, rename = "Id")]
    id: String,
    #[serde(default, rename = "id")]
    id_lower: String,
}

#[derive(Debug, Deserialize)]
struct KeyListResponse {
    #[serde(default, rename = "Keys")]
    keys: Vec<KeyListEntry>,
}

#[derive(Debug, Deserialize)]
struct AddResponse {
    #[serde(rename = "Hash")]
    hash: String,
}

/// Add raw bytes to IPFS via /api/v0/add, pin the result, and return the CID.
pub async fn ipfs_add(kubo_url: &str, data: Vec<u8>) -> Result<String> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/add");

    let part = multipart::Part::bytes(data).file_name("data");
    let form = multipart::Form::new().part("file", part);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let body = client
        .post(url)
        .query(&[("pin", "true")])
        .multipart(form)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let parsed: AddResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing add response: {} body={}", e, body))?;

    Ok(parsed.hash)
}

/// Fetch raw bytes from IPFS by CID (via /api/v0/cat).
pub async fn cat_cid(kubo_url: &str, cid: &str) -> Result<String> {    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/cat");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let body = client
        .post(url)
        .query(&[("arg", cid)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    Ok(body)
}

pub async fn dag_put_dag_cbor<T>(kubo_url: &str, value: &T) -> Result<String>
where
    T: Serialize,
{
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/dag/put");
    let payload = serde_json::to_vec(value)?;

    let part = multipart::Part::bytes(payload)
        .file_name("node.json")
        .mime_str("application/json")?;
    let form = multipart::Form::new().part("file", part);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let response = client
        .post(url)
        .query(&[
            ("store-codec", "dag-cbor"),
            ("input-codec", "dag-json"),
            ("pin", "true"),
        ])
        .multipart(form)
        .send()
        .await?
        .error_for_status()?;

    let body = response.text().await?;
    let parsed: DagPutResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing dag/put response: {} body={}", e, body))?;

    let cid = parsed
        .cid_upper
        .or(parsed.cid)
        .map(|c| c.slash)
        .ok_or_else(|| anyhow!("missing CID in dag/put response: {}", body))?;
    Ok(cid)
}

pub async fn dag_get_dag_cbor<T>(kubo_url: &str, cid: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/dag/get");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let body = client
        .post(url)
        .query(&[("arg", cid)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    serde_json::from_str::<T>(&body)
        .map_err(|e| anyhow!("failed parsing dag/get response for {}: {} body={}", cid, e, body))
}

#[allow(dead_code)]
pub async fn ipns_publish(kubo_url: &str, key_name: &str, cid: &str) -> Result<String> {
    let options = IpnsPublishOptions::default();
    ipns_publish_with_options(kubo_url, key_name, cid, &options).await
}

#[derive(Clone, Debug)]
pub struct IpnsPublishOptions {
    pub timeout: Duration,
    pub allow_offline: bool,
    pub lifetime: String,
    pub resolve: bool,
    pub quieter: bool,
}

impl Default for IpnsPublishOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            allow_offline: true,
            lifetime: "8760h".to_string(),
            resolve: false,
            quieter: true,
        }
    }
}

pub async fn ipns_publish_with_options(
    kubo_url: &str,
    key_name: &str,
    cid: &str,
    options: &IpnsPublishOptions,
) -> Result<String> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/name/publish");
    let arg = format!("/ipfs/{cid}");

    let client = reqwest::Client::builder()
        .timeout(options.timeout)
        .build()?;

    let allow_offline = if options.allow_offline { "true" } else { "false" };
    let resolve = if options.resolve { "true" } else { "false" };
    let quieter = if options.quieter { "true" } else { "false" };

    let response = client
        .post(url)
        .query(&[
            ("arg", arg.as_str()),
            ("key", key_name),
            ("allow-offline", allow_offline),
            ("lifetime", options.lifetime.as_str()),
            ("resolve", resolve),
            ("quieter", quieter),
        ])
        .send()
        .await?
        .error_for_status()?;

    let body = response.text().await?;
    let parsed: NamePublishResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing name/publish response: {} body={}", e, body))?;
    let value = if !parsed.value_upper.is_empty() {
        parsed.value_upper
    } else {
        parsed.value_lower
    };
    if value.is_empty() {
        return Err(anyhow!("missing value in name/publish response: {}", body));
    }
    Ok(value)
}

pub async fn ipns_publish_with_retry(
    kubo_url: &str,
    key_name: &str,
    cid: &str,
    options: &IpnsPublishOptions,
    attempts: u32,
    initial_backoff: Duration,
) -> Result<String> {
    if attempts == 0 {
        return Err(anyhow!("ipns publish attempts must be >= 1"));
    }

    let mut backoff = initial_backoff;
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=attempts {
        match ipns_publish_with_options(kubo_url, key_name, cid, options).await {
            Ok(value) => return Ok(value),
            Err(err) => {
                warn!(
                    "IPNS publish attempt {}/{} failed for key '{}' and cid '{}': {}",
                    attempt,
                    attempts,
                    key_name,
                    cid,
                    err
                );
                last_err = Some(err);
                if attempt < attempts {
                    sleep(backoff).await;
                    let doubled = backoff.as_millis().saturating_mul(2);
                    let cap = 30_000u128;
                    let next_ms = std::cmp::min(doubled, cap) as u64;
                    backoff = Duration::from_millis(next_ms);
                }
            }
        }
    }

    Err(anyhow!(
        "IPNS publish failed after {} attempt(s): {}",
        attempts,
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    ))
}

pub async fn name_resolve(kubo_url: &str, path: &str, recursive: bool) -> Result<String> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/name/resolve");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let recursive_flag = if recursive { "true" } else { "false" };
    let response = client
        .post(url)
        .query(&[("arg", path), ("recursive", recursive_flag)])
        .send()
        .await?
        .error_for_status()?;

    let body = response.text().await?;
    let parsed: NameResolveResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing name/resolve response: {} body={}", e, body))?;
    let resolved = if !parsed.path_upper.is_empty() {
        parsed.path_upper
    } else {
        parsed.path_lower
    };
    if resolved.is_empty() {
        return Err(anyhow!("missing path in name/resolve response: {}", body));
    }
    Ok(resolved)
}

pub async fn wait_for_kubo_api(
    kubo_url: &str,
    attempts: u32,
    initial_backoff: Duration,
) -> Result<()> {
    if attempts == 0 {
        return Err(anyhow!("kubo readiness attempts must be >= 1"));
    }

    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/version");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()?;

    let mut backoff = initial_backoff;
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=attempts {
        let result = async {
            let response = client.post(&url).send().await?.error_for_status()?;
            let body = response.text().await?;
            let parsed: VersionResponse = serde_json::from_str(&body)
                .map_err(|e| anyhow!("failed parsing version response: {} body={}", e, body))?;
            let version = if !parsed.version_upper.is_empty() {
                parsed.version_upper
            } else {
                parsed.version_lower
            };
            if version.trim().is_empty() {
                return Err(anyhow!("missing version field in response: {}", body));
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => return Ok(()),
            Err(err) => {
                warn!(
                    "Kubo API readiness attempt {}/{} failed for {}: {}",
                    attempt,
                    attempts,
                    kubo_url,
                    err
                );
                last_err = Some(err);
                if attempt < attempts {
                    sleep(backoff).await;
                    let doubled = backoff.as_millis().saturating_mul(2);
                    let cap = 30_000u128;
                    let next_ms = std::cmp::min(doubled, cap) as u64;
                    backoff = Duration::from_millis(next_ms);
                }
            }
        }
    }

    Err(anyhow!(
        "Kubo API not ready after {} attempt(s): {}",
        attempts,
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    ))
}

pub async fn list_kubo_key_names(kubo_url: &str) -> Result<Vec<String>> {
    let keys = list_kubo_keys(kubo_url).await?;
    Ok(keys.into_iter().map(|k| k.name).collect())
}

#[derive(Clone, Debug)]
pub struct KuboKey {
    pub name: String,
    pub id: String,
}

pub async fn list_kubo_keys(kubo_url: &str) -> Result<Vec<KuboKey>> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/key/list");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let response = client
        .post(url)
        .send()
        .await?
        .error_for_status()?;

    let body = response.text().await?;
    let parsed: KeyListResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("failed parsing key/list response: {} body={}", e, body))?;
    Ok(parsed
        .keys
        .into_iter()
        .filter_map(|k| {
            let name = if !k.name.trim().is_empty() {
                k.name.trim().to_string()
            } else {
                k.name_lower.trim().to_string()
            };
            let id = if !k.id.trim().is_empty() {
                k.id.trim().to_string()
            } else {
                k.id_lower.trim().to_string()
            };
            if name.is_empty() {
                return None;
            }
            Some(KuboKey { name, id })
        })
        .collect())
}

pub async fn generate_kubo_key(kubo_url: &str, key_name: &str) -> Result<()> {
    let base = kubo_url.trim_end_matches('/');
    let url = format!("{base}/api/v0/key/gen");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    client
        .post(url)
        .query(&[("arg", key_name), ("type", "ed25519")])
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}
