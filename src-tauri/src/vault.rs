// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Master-password vault. A 256-bit key is derived with Argon2id and used to encrypt
//! each secret with XChaCha20-Poly1305. Only the salt and an encrypted check value are
//! stored; the key exists only in memory while the vault is unlocked.

use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::error::{AppError, AppResult};
use crate::store::Store;

const SALT_KEY: &str = "vault.salt";
const CHECK_KEY: &str = "vault.check";
const CHECK_PLAINTEXT: &[u8] = b"bunnylink-vault-v1";
const NONCE_LEN: usize = 24;

#[derive(Default)]
pub struct Vault {
    key: Mutex<Option<[u8; 32]>>,
}

fn random<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf
}

fn derive(password: &str, salt: &[u8]) -> AppResult<[u8; 32]> {
    // OWASP recommended Argon2id parameters: 19 MiB, 2 iterations, 1 lane.
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|e| AppError::Other(e.to_string()))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::Other(e.to_string()))?;
    Ok(key)
}

fn seal(key: &[u8; 32], plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce_bytes = random::<NONCE_LEN>();
    let nonce = XNonce::from(nonce_bytes);
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| AppError::Other("Encryption failed.".into()))?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

fn open(key: &[u8; 32], blob: &[u8]) -> AppResult<Vec<u8>> {
    if blob.len() < NONCE_LEN {
        return Err(AppError::Other("Corrupt secret.".into()));
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().unwrap();
    XChaCha20Poly1305::new(key.into())
        .decrypt(&XNonce::from(nonce), ct)
        .map_err(|_| AppError::WrongPassword)
}

impl Vault {
    pub fn is_initialized(store: &Store) -> AppResult<bool> {
        Ok(store.meta(SALT_KEY)?.is_some())
    }

    pub fn is_unlocked(&self) -> bool {
        self.key.lock().unwrap().is_some()
    }

    pub fn create(&self, store: &Store, password: &str) -> AppResult<()> {
        if Self::is_initialized(store)? {
            return Err(AppError::Invalid("A vault already exists.".into()));
        }
        if password.chars().count() < 8 {
            return Err(AppError::Invalid("Use at least 8 characters for the master password.".into()));
        }
        let salt = random::<16>();
        let key = derive(password, &salt)?;
        store.set_meta(SALT_KEY, &salt)?;
        store.set_meta(CHECK_KEY, &seal(&key, CHECK_PLAINTEXT)?)?;
        *self.key.lock().unwrap() = Some(key);
        Ok(())
    }

    pub fn unlock(&self, store: &Store, password: &str) -> AppResult<()> {
        let salt = store.meta(SALT_KEY)?.ok_or(AppError::Invalid("No vault exists yet.".into()))?;
        let check = store.meta(CHECK_KEY)?.ok_or(AppError::Invalid("Vault is damaged.".into()))?;
        let key = derive(password, &salt)?;
        if open(&key, &check)? != CHECK_PLAINTEXT {
            return Err(AppError::WrongPassword);
        }
        *self.key.lock().unwrap() = Some(key);
        Ok(())
    }

    pub fn lock(&self) {
        if let Some(mut k) = self.key.lock().unwrap().take() {
            k.fill(0);
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let guard = self.key.lock().unwrap();
        seal(guard.as_ref().ok_or(AppError::VaultLocked)?, plaintext)
    }

    pub fn decrypt(&self, blob: &[u8]) -> AppResult<Vec<u8>> {
        let guard = self.key.lock().unwrap();
        open(guard.as_ref().ok_or(AppError::VaultLocked)?, blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_lock_unlock_roundtrip() {
        let store = Store::in_memory().unwrap();
        let v = Vault::default();
        v.create(&store, "correct horse").unwrap();
        let blob = v.encrypt(b"s3cret").unwrap();
        v.lock();
        assert!(matches!(v.decrypt(&blob), Err(AppError::VaultLocked)));
        assert!(matches!(v.unlock(&store, "wrong password"), Err(AppError::WrongPassword)));
        v.unlock(&store, "correct horse").unwrap();
        assert_eq!(v.decrypt(&blob).unwrap(), b"s3cret");
    }
}
