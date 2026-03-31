use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Result, anyhow};
use nanoid::nanoid;

use crate::kubo::{
    IpnsPublishOptions, generate_kubo_key, ipfs_add, ipns_publish_with_retry, list_kubo_key_names,
    list_kubo_keys, name_resolve,
};

fn package_actor_web_directory(source_dir: &Path) -> Result<Vec<u8>> {
    let index = source_dir.join("index.html");
    if !index.exists() {
        return Err(anyhow!(
            "actor web source dir {} is missing index.html",
            source_dir.display()
        ));
    }

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut tar_builder = tar::Builder::new(&mut cursor);
        tar_builder.append_dir_all(".", source_dir).map_err(|e| {
            anyhow!(
                "failed creating actor web archive from {}: {}",
                source_dir.display(),
                e
            )
        })?;
        tar_builder.finish().map_err(|e| {
            anyhow!(
                "failed finalizing actor web archive from {}: {}",
                source_dir.display(),
                e
            )
        })?;
    }
    Ok(cursor.into_inner())
}

async fn ensure_kubo_key_exists(kubo_url: &str, key_name: &str) -> Result<()> {
    let key_names = list_kubo_key_names(kubo_url).await?;
    if key_names.iter().any(|name| name == key_name) {
        return Ok(());
    }
    generate_kubo_key(kubo_url, key_name).await
}

pub async fn publish_actor_web_from_dir(
    kubo_url: &str,
    source_dir: &Path,
    ipns_key_name: Option<&str>,
) -> Result<String> {
    let archive = package_actor_web_directory(source_dir)?;
    let cid = ipfs_add(kubo_url, archive).await?;

    if let Some(key_name) = ipns_key_name {
        ensure_kubo_key_exists(kubo_url, key_name).await?;
        let options = IpnsPublishOptions::default();
        ipns_publish_with_retry(
            kubo_url,
            key_name,
            &cid,
            &options,
            3,
            Duration::from_millis(500),
        )
        .await?;
    }

    Ok(cid)
}

pub async fn materialize_actor_web_from_cid(
    kubo_url: &str,
    cid: &str,
    cache_root: &Path,
) -> Result<PathBuf> {
    fs::create_dir_all(cache_root).map_err(|e| {
        anyhow!(
            "failed creating actor web cache root {}: {}",
            cache_root.display(),
            e
        )
    })?;

    let cid_clean = cid.trim();
    if cid_clean.is_empty() {
        return Err(anyhow!("actor web cid is empty"));
    }

    let stage_dir = cache_root.join(format!(".stage-{}-{}", cid_clean, nanoid!(8)));
    let target_dir = cache_root.join(cid_clean);
    fs::create_dir_all(&stage_dir).map_err(|e| {
        anyhow!(
            "failed creating actor web stage dir {}: {}",
            stage_dir.display(),
            e
        )
    })?;

    let base = kubo_url.trim_end_matches('/');
    let url = format!(
        "{}/api/v0/get?arg={}&archive=true&compress=false",
        base, cid_clean
    );
    let response = reqwest::Client::new()
        .post(&url)
        .send()
        .await
        .map_err(|e| anyhow!("failed downloading actor web cid {}: {}", cid_clean, e))?
        .error_for_status()
        .map_err(|e| anyhow!("failed downloading actor web cid {}: {}", cid_clean, e))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| anyhow!("failed reading actor web archive for {}: {}", cid_clean, e))?;

    let mut archive = tar::Archive::new(Cursor::new(bytes));
    archive
        .unpack(&stage_dir)
        .map_err(|e| anyhow!("failed unpacking actor web archive for {}: {}", cid_clean, e))?;

    let extracted_root = find_index_html_root(&stage_dir)
        .ok_or_else(|| anyhow!("actor web archive for {} does not contain index.html", cid_clean))?;

    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)
            .map_err(|e| anyhow!("failed clearing actor web cache {}: {}", target_dir.display(), e))?;
    }

    if extracted_root == stage_dir {
        fs::rename(&stage_dir, &target_dir).map_err(|e| {
            anyhow!(
                "failed moving actor web stage {} -> {}: {}",
                stage_dir.display(),
                target_dir.display(),
                e
            )
        })?;
    } else {
        fs::rename(&extracted_root, &target_dir).map_err(|e| {
            anyhow!(
                "failed moving actor web dir {} -> {}: {}",
                extracted_root.display(),
                target_dir.display(),
                e
            )
        })?;
        let _ = fs::remove_dir_all(&stage_dir);
    }

    Ok(target_dir)
}

fn find_index_html_root(dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join("index.html");
    if candidate.exists() {
        return Some(dir.to_path_buf());
    }

    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_index_html_root(&path) {
                return Some(found);
            }
        }
    }

    None
}

async fn resolve_key_id_by_name(kubo_url: &str, key_name: &str) -> Result<String> {
    let key_name = key_name.trim();
    if key_name.is_empty() {
        return Err(anyhow!("kubo key name must not be empty"));
    }

    let keys = list_kubo_keys(kubo_url).await?;
    keys.into_iter()
        .find(|key| key.name == key_name)
        .map(|key| key.id)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| anyhow!("kubo key '{}' exists but has no usable id", key_name))
}

pub async fn resolve_actor_web_cid_from_ipns_key(
    kubo_url: &str,
    key_name: &str,
) -> Result<Option<String>> {
    let key_name = key_name.trim();
    if key_name.is_empty() {
        return Ok(None);
    }

    let key_id = match resolve_key_id_by_name(kubo_url, key_name).await {
        Ok(id) => id,
        Err(_) => return Ok(None),
    };
    let resolved = name_resolve(kubo_url, &format!("/ipns/{}", key_id), true).await?;
    let Some(rest) = resolved.strip_prefix("/ipfs/") else {
        return Ok(None);
    };
    let cid = rest.split('/').next().unwrap_or_default().trim();
    if cid.is_empty() {
        Ok(None)
    } else {
        Ok(Some(cid.to_string()))
    }
}