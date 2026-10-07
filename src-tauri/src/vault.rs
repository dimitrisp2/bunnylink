// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Master-password vault. A 256-bit key is derived with Argon2id and used to encrypt
//! each secret with XChaCha20-Poly1305. Only the salt and an encrypted check value are
//! stored; the key exists only in memory while the vault is unlocked.
//!
//! The key lives in one fixed allocation that is wiped on lock; on Windows it is a page of
//! its own, kept out of the page file and crash reports. Decrypted plaintext is returned
//! as `Zeroizing`, so it is wiped as soon as the caller is done with it.

use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::store::Store;

const SALT_KEY: &str = "vault.salt";
const CHECK_KEY: &str = "vault.check";
const CHECK_PLAINTEXT: &[u8] = b"bunnylink-vault-v1";
const NONCE_LEN: usize = 24;

/// The key never moves (a move would leave a stale copy) and is wiped on drop.
type Key = key_memory::KeyBuf;

mod key_memory {
    use std::ops::{Deref, DerefMut};
    use zeroize::Zeroize;

    /// A page of its own, allocated zeroed, locked in RAM so it is never written to the
    /// page file, and excluded from Windows Error Reporting crash dumps. Failing to lock or
    /// exclude only loses that extra protection; the key still works and is still wiped.
    #[cfg(windows)]
    pub struct KeyBuf(std::ptr::NonNull<[u8; 32]>);

    #[cfg(windows)]
    const PAGE: usize = 4096;

    // SAFETY: the page belongs to this value alone, and access goes through &self / &mut self.
    #[cfg(windows)]
    unsafe impl Send for KeyBuf {}
    #[cfg(windows)]
    unsafe impl Sync for KeyBuf {}

    #[cfg(windows)]
    impl KeyBuf {
        pub fn new() -> Self {
            use windows_sys::Win32::System::ErrorReporting::WerRegisterExcludedMemoryBlock;
            use windows_sys::Win32::System::Memory::{VirtualAlloc, VirtualLock, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
            // SAFETY: asks for a fresh committed page; a null result is handled below.
            let page = unsafe { VirtualAlloc(std::ptr::null(), PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
            let ptr = std::ptr::NonNull::new(page.cast::<[u8; 32]>()).expect("no memory for the vault key");
            // SAFETY: `page` is PAGE bytes we own until Drop releases them.
            unsafe {
                VirtualLock(page, PAGE);
                WerRegisterExcludedMemoryBlock(page, PAGE as u32);
            }
            Self(ptr)
        }
    }

    #[cfg(windows)]
    impl Drop for KeyBuf {
        fn drop(&mut self) {
            use windows_sys::Win32::System::ErrorReporting::WerUnregisterExcludedMemoryBlock;
            use windows_sys::Win32::System::Memory::{VirtualFree, VirtualUnlock, MEM_RELEASE};
            self.zeroize();
            let page = self.0.as_ptr().cast();
            // SAFETY: the page came from VirtualAlloc in `new` and is not used after this.
            unsafe {
                WerUnregisterExcludedMemoryBlock(page);
                VirtualUnlock(page, PAGE);
                VirtualFree(page, 0, MEM_RELEASE);
            }
        }
    }

    #[cfg(windows)]
    impl Deref for KeyBuf {
        type Target = [u8; 32];
        fn deref(&self) -> &[u8; 32] {
            // SAFETY: the pointer is valid, aligned and initialised (VirtualAlloc zeroes) for our lifetime.
            unsafe { self.0.as_ref() }
        }
    }

    #[cfg(windows)]
    impl DerefMut for KeyBuf {
        fn deref_mut(&mut self) -> &mut [u8; 32] {
            // SAFETY: as in deref; &mut self makes this the only reference.
            unsafe { self.0.as_mut() }
        }
    }

    /// Elsewhere: a plain heap allocation, still fixed in place and wiped on drop.
    #[cfg(not(windows))]
    pub struct KeyBuf(Box<[u8; 32]>);

    #[cfg(not(windows))]
    impl KeyBuf {
        pub fn new() -> Self {
            Self(Box::new([0u8; 32]))
        }
    }

    #[cfg(not(windows))]
    impl Drop for KeyBuf {
        fn drop(&mut self) {
            self.zeroize();
        }
    }

    #[cfg(not(windows))]
    impl Deref for KeyBuf {
        type Target = [u8; 32];
        fn deref(&self) -> &[u8; 32] {
            &self.0
        }
    }

    #[cfg(not(windows))]
    impl DerefMut for KeyBuf {
        fn deref_mut(&mut self) -> &mut [u8; 32] {
            &mut self.0
        }
    }

    impl Zeroize for KeyBuf {
        fn zeroize(&mut self) {
            self.deref_mut().zeroize();
        }
    }
}

#[derive(Default)]
pub struct Vault {
    key: Mutex<Option<Key>>,
}

fn random<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf
}

fn derive(password: &str, salt: &[u8]) -> AppResult<Key> {
    // OWASP recommended Argon2id parameters: 19 MiB, 2 iterations, 1 lane.
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|e| AppError::Other(e.to_string()))?;
    // Derived straight into its final place; argon2's zeroize feature wipes its working memory.
    let mut key = Key::new();
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key[..])
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

fn open(key: &[u8; 32], blob: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
    if blob.len() < NONCE_LEN {
        return Err(AppError::Other("Corrupt secret.".into()));
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().unwrap();
    XChaCha20Poly1305::new(key.into())
        .decrypt(&XNonce::from(nonce), ct)
        .map(Zeroizing::new)
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
        if open(&key, &check)?.as_slice() != CHECK_PLAINTEXT {
            return Err(AppError::WrongPassword);
        }
        *self.key.lock().unwrap() = Some(key);
        Ok(())
    }

    pub fn lock(&self) {
        // Dropping the key wipes it.
        self.key.lock().unwrap().take();
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let guard = self.key.lock().unwrap();
        seal(guard.as_ref().ok_or(AppError::VaultLocked)?, plaintext)
    }

    pub fn decrypt(&self, blob: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
        let guard = self.key.lock().unwrap();
        open(guard.as_ref().ok_or(AppError::VaultLocked)?, blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_memory_starts_zeroed_and_is_reusable() {
        // Many lock/unlock cycles each take and release a page; a bad free would crash here.
        for i in 0..2000u32 {
            let mut k = Key::new();
            assert_eq!(*k, [0u8; 32]);
            k[..4].copy_from_slice(&i.to_le_bytes());
            assert_eq!(k[..4], i.to_le_bytes());
        }
    }

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
        assert_eq!(v.decrypt(&blob).unwrap().as_slice(), b"s3cret");
    }
}
