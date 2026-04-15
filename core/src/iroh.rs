use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use iroh::SecretKey;
use rand::RngCore;

use crate::secure_fs::{SecureFileKind, write_secure_file};

/// Convert a socket address to a multiaddr string (QUIC-v1 over UDP).
pub fn socket_addr_to_multiaddr(addr: &SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) => format!("/ip4/{}/udp/{}/quic-v1", ip, addr.port()),
        IpAddr::V6(ip) => format!("/ip6/{}/udp/{}/quic-v1", ip, addr.port()),
    }
}

/// Load an iroh secret key from a 32-byte file on disk.
pub fn load_persisted_iroh_secret_key(path: &PathBuf) -> Result<Option<SecretKey>> {
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(path)?;
    let key_bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid iroh secret key file length in {}", path.display()))?;

    Ok(Some(SecretKey::from_bytes(&key_bytes)))
}

/// Generate a new random iroh secret key and write it to disk.
pub fn generate_iroh_secret_file(path: &PathBuf) -> Result<()> {
    if path.exists() {
        return Err(anyhow!("iroh secret already exists at {}", path.display()));
    }

    let mut key_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key_bytes);
    write_secure_file(path, &key_bytes, SecureFileKind::IrohSecret)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn repo_tmp_file(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tmp")
            .join("core-tests");
        fs::create_dir_all(&root).expect("failed creating repo tmp test directory");
        root.join(name)
    }

    #[test]
    fn multiaddr_ipv4() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 4433);
        assert_eq!(socket_addr_to_multiaddr(&addr), "/ip4/127.0.0.1/udp/4433/quic-v1");
    }

    #[test]
    fn multiaddr_ipv6() {
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 5555);
        assert_eq!(socket_addr_to_multiaddr(&addr), "/ip6/::1/udp/5555/quic-v1");
    }

    #[test]
    fn load_missing_key_returns_none() {
        let path = repo_tmp_file("ma-test-nonexistent-iroh-key");
        let _ = fs::remove_file(&path);
        assert!(load_persisted_iroh_secret_key(&path).unwrap().is_none());
    }

    #[test]
    fn generate_refuses_existing() {
        let path = repo_tmp_file("ma-test-iroh-gen-existing");
        let _ = fs::remove_file(&path);
        fs::write(&path, b"x").ok();
        let result = generate_iroh_secret_file(&path);
        assert!(result.is_err());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn roundtrip_generate_and_load() {
        let path = repo_tmp_file("ma-test-iroh-roundtrip-key");
        let _ = fs::remove_file(&path);
        generate_iroh_secret_file(&path).unwrap();
        let key = load_persisted_iroh_secret_key(&path).unwrap();
        assert!(key.is_some());
        fs::remove_file(&path).ok();
    }
}
