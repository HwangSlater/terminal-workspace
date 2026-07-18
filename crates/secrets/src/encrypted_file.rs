//! Local AES-256-GCM encrypted secret storage — the fallback for
//! environments with no OS keyring backend (headless Linux, some
//! containers/CI). See `docs/04-extensions/security.md` §1 and
//! `step7.md` for why this exists alongside [`crate::KeyringProvider`]
//! rather than instead of it, and its honest limitation.

use crate::{SecretProvider, SecretWriter};
use aes_gcm::aead::{Aead, Generate, KeyInit};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use async_trait::async_trait;
use common::{Result, WorkspaceError};
use secrecy::SecretString;
use std::collections::HashMap;
use std::path::PathBuf;

const KEY_FILE: &str = "secrets.key";
const VAULT_FILE: &str = "secrets.enc";
const NONCE_LEN: usize = 12;

/// AES-256-GCM-encrypted local file secret storage.
///
/// **Honest limitation**: the encryption key lives in a plain file
/// (`secrets.key`) next to the ciphertext (`secrets.enc`), both under
/// `dir`. With no OS keyring backing it, this protects against casual
/// exposure (e.g. `secrets.enc` alone leaking into a backup or sync) but
/// **not** a determined attacker with full filesystem read access on this
/// machine — it is a fallback for when [`crate::KeyringProvider`] is
/// unavailable, not a security-equivalent alternative to it.
pub struct EncryptedFileProvider {
    dir: PathBuf,
}

impl EncryptedFileProvider {
    /// Create a provider storing its vault under `dir` (created on first
    /// write if it doesn't exist).
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join(KEY_FILE)
    }

    fn vault_path(&self) -> PathBuf {
        self.dir.join(VAULT_FILE)
    }

    fn load_or_create_key(&self) -> Result<Key<Aes256Gcm>> {
        if let Ok(bytes) = std::fs::read(self.key_path()) {
            if let Ok(key) = Key::<Aes256Gcm>::try_from(bytes.as_slice()) {
                return Ok(key);
            }
        }
        let key = Key::<Aes256Gcm>::generate();
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| WorkspaceError::Security(format!("Failed to create secrets dir: {e}")))?;
        std::fs::write(self.key_path(), key.as_slice())
            .map_err(|e| WorkspaceError::Security(format!("Failed to write secrets key: {e}")))?;
        restrict_permissions(&self.key_path())?;
        Ok(key)
    }

    /// Empty map (not an error) when the vault doesn't exist yet — "no
    /// secrets stored" is the normal first-run state, not a failure.
    fn load_vault(&self) -> Result<HashMap<String, String>> {
        let Ok(data) = std::fs::read(self.vault_path()) else {
            return Ok(HashMap::new());
        };
        if data.len() < NONCE_LEN {
            return Ok(HashMap::new());
        }
        let key = self.load_or_create_key()?;
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let nonce = Nonce::<<Aes256Gcm as AeadCore>::NonceSize>::try_from(nonce_bytes)
            .map_err(|_| WorkspaceError::Security("Corrupt secrets vault (bad nonce)".into()))?;
        let cipher = Aes256Gcm::new(&key);
        let plaintext = cipher.decrypt(&nonce, ciphertext).map_err(|_| {
            WorkspaceError::Security("Failed to decrypt local secrets vault".into())
        })?;
        serde_json::from_slice(&plaintext)
            .map_err(|e| WorkspaceError::Security(format!("Corrupt secrets vault: {e}")))
    }

    fn save_vault(&self, map: &HashMap<String, String>) -> Result<()> {
        let key = self.load_or_create_key()?;
        let plaintext = serde_json::to_vec(map)
            .map_err(|e| WorkspaceError::Security(format!("Failed to serialize vault: {e}")))?;
        let cipher = Aes256Gcm::new(&key);
        let nonce = Nonce::<<Aes256Gcm as AeadCore>::NonceSize>::generate();
        let ciphertext = cipher.encrypt(&nonce, plaintext.as_ref()).map_err(|_| {
            WorkspaceError::Security("Failed to encrypt local secrets vault".into())
        })?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| WorkspaceError::Security(format!("Failed to create secrets dir: {e}")))?;
        std::fs::write(self.vault_path(), out)
            .map_err(|e| WorkspaceError::Security(format!("Failed to write secrets vault: {e}")))?;
        restrict_permissions(&self.vault_path())
    }
}

/// Without this, `secrets.key`/`secrets.enc` are written with the process's
/// default umask on Unix — typically group/world-readable, meaning any
/// other local account could read the encryption key and decrypt the
/// vault. This doesn't make the fallback storage equivalent to a real OS
/// keyring (see this module's doc comment), but there's no reason to leave
/// it weaker than a one-line `chmod` fixes.
#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        WorkspaceError::Security(format!("Failed to restrict secrets file permissions: {e}"))
    })
}

/// Windows ACLs on a user's own profile directory already default to
/// owner-only access, so there's no equivalent single-call tightening to
/// apply here.
#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[async_trait]
impl SecretProvider for EncryptedFileProvider {
    async fn get_secret(&self, key: &str) -> Result<Option<SecretString>> {
        let map = self.load_vault()?;
        Ok(map.get(key).cloned().map(SecretString::from))
    }
}

#[async_trait]
impl SecretWriter for EncryptedFileProvider {
    async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        let mut map = self.load_vault()?;
        map.insert(key.to_string(), value.to_string());
        self.save_vault(&map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn temp_dir(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tw_secrets_test_{test_name}_{}",
            uuid::Uuid::new_v4()
        ))
    }

    #[tokio::test]
    async fn missing_vault_returns_none_not_error() {
        let provider = EncryptedFileProvider::new(temp_dir("missing_vault"));
        let secret = provider.get_secret("SOME_KEY").await.unwrap();
        assert!(secret.is_none());
    }

    #[tokio::test]
    async fn round_trips_a_secret_through_the_encrypted_vault() {
        let provider = EncryptedFileProvider::new(temp_dir("round_trip"));
        provider
            .set_secret("SLACK_BOT_TOKEN", "xoxb-test")
            .await
            .unwrap();
        let secret = provider.get_secret("SLACK_BOT_TOKEN").await.unwrap();
        assert_eq!(secret.unwrap().expose_secret(), "xoxb-test");
    }

    #[tokio::test]
    async fn the_vault_file_on_disk_does_not_contain_the_plaintext_secret() {
        let dir = temp_dir("plaintext_check");
        let provider = EncryptedFileProvider::new(dir.clone());
        provider
            .set_secret("SLACK_BOT_TOKEN", "xoxb-super-secret-value")
            .await
            .unwrap();
        let raw = std::fs::read(dir.join(VAULT_FILE)).unwrap();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(!raw_str.contains("xoxb-super-secret-value"));
    }

    #[tokio::test]
    async fn setting_a_second_key_preserves_the_first() {
        let provider = EncryptedFileProvider::new(temp_dir("preserve"));
        provider.set_secret("KEY_A", "value-a").await.unwrap();
        provider.set_secret("KEY_B", "value-b").await.unwrap();
        assert_eq!(
            provider
                .get_secret("KEY_A")
                .await
                .unwrap()
                .unwrap()
                .expose_secret(),
            "value-a"
        );
        assert_eq!(
            provider
                .get_secret("KEY_B")
                .await
                .unwrap()
                .unwrap()
                .expose_secret(),
            "value-b"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn vault_and_key_files_are_not_group_or_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("permissions");
        let provider = EncryptedFileProvider::new(dir.clone());
        provider.set_secret("KEY", "value").await.unwrap();

        for file in [KEY_FILE, VAULT_FILE] {
            let mode = std::fs::metadata(dir.join(file))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "{file} must not be group/world readable (mode was {mode:o})"
            );
        }
    }
}
