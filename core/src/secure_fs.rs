use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow};

#[derive(Clone, Copy, Debug)]
pub enum SecureFileKind {
    RuntimeConfig,
    IrohSecret,
    SensitiveData,
}

pub fn write_secure_file(path: &Path, bytes: &[u8], kind: SecureFileKind) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if parent.exists() {
                fs::create_dir_all(parent).map_err(|err| {
                    anyhow!("failed creating directory {}: {}", parent.display(), err)
                })?;
            } else {
                ensure_private_dir(parent)?;
            }
        }
    }

    fs::write(path, bytes)
        .map_err(|err| anyhow!("failed writing {}: {}", path.display(), err))?;

    apply_secure_permissions(path, kind)
}

pub fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|err| anyhow!("failed creating directory {}: {}", path.display(), err))?;
    apply_private_dir_permissions(path)
}

#[cfg(unix)]
fn apply_secure_permissions(path: &Path, kind: SecureFileKind) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = match kind {
        SecureFileKind::RuntimeConfig | SecureFileKind::SensitiveData => 0o600,
        SecureFileKind::IrohSecret => 0o400,
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|err| {
        anyhow!(
            "failed setting permissions {:o} on {}: {}",
            mode,
            path.display(),
            err
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn apply_private_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|err| {
        anyhow!(
            "failed setting directory permissions 0700 on {}: {}",
            path.display(),
            err
        )
    })?;
    Ok(())
}

#[cfg(windows)]
fn apply_secure_permissions(path: &Path, _kind: SecureFileKind) -> Result<()> {
    apply_windows_current_user_acl(path)
}

#[cfg(windows)]
fn apply_private_dir_permissions(path: &Path) -> Result<()> {
    apply_windows_current_user_acl(path)
}

#[cfg(windows)]
fn apply_windows_current_user_acl(path: &Path) -> Result<()> {
    use windows_acl::acl::ACL;
    use windows_acl::helper::{current_user, name_to_sid, string_to_sid};
    use winapi::um::winnt::{FILE_ALL_ACCESS, PSID};

    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("path is not valid Unicode: {}", path.display()))?;

    let mut acl = ACL::from_file_path(path_str, false)
        .map_err(|code| anyhow!("failed loading ACL for {}: windows error {}", path.display(), code))?;

    let current_user_name = current_user()
        .map_err(|code| anyhow!("failed resolving current user name: windows error {}", code))?;
    let current_user_sid = name_to_sid(&current_user_name).map_err(|code| {
        anyhow!(
            "failed resolving SID for current user '{}': windows error {}",
            current_user_name,
            code
        )
    })?;

    // Remove broad group entries before granting current-user full access.
    for sid_text in ["S-1-1-0", "S-1-5-11", "S-1-5-32-545"] {
        let sid = string_to_sid(sid_text)
            .map_err(|code| anyhow!("failed converting SID {}: windows error {}", sid_text, code))?;
        acl.remove(sid.as_ptr() as PSID, None, None).map_err(|code| {
            anyhow!(
                "failed removing broad ACL entry {} on {}: windows error {}",
                sid_text,
                path.display(),
                code
            )
        })?;
    }

    let current_user_set = acl
        .allow(current_user_sid.as_ptr() as PSID, false, FILE_ALL_ACCESS)
        .map_err(|code| {
            anyhow!(
                "failed granting current-user ACL on {}: windows error {}",
                path.display(),
                code
            )
        })?;
    if !current_user_set {
        return Err(anyhow!(
            "failed granting current-user ACL on {}: operation returned false",
            path.display()
        ));
    }

    // Fail hard if broad group SIDs still have allow entries on the target object.
    let entries = acl
        .all()
        .map_err(|code| anyhow!("failed re-reading ACL on {}: windows error {}", path.display(), code))?;
    for entry in entries {
        let broad_sid = matches!(
            entry.string_sid.as_str(),
            "S-1-1-0" | "S-1-5-11" | "S-1-5-32-545"
        );
        if broad_sid {
            return Err(anyhow!(
                "insecure ACL on {}: broad SID {} still present",
                path.display(),
                entry.string_sid
            ));
        }
    }

    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn apply_secure_permissions(_path: &Path, _kind: SecureFileKind) -> Result<()> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn apply_private_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}
