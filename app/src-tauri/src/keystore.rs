//! Where the keys of entries the user added by hand are kept.
//!
//! First choice is the system keychain (macOS Keychain, Windows Credential Manager,
//! the Secret Service on Linux). Only if that is not available does the key go into
//! a private file (0600) in the app's config folder. Keys are never logged.
//!
//! SPDX-License-Identifier: MIT

use crate::settings::{self, AppError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SERVICE: &str = "wickrun-brain";

/// Where one key ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Store {
    #[default]
    Keychain,
    File,
}

/// A secret store the app can try first. The real one is the OS keychain.
pub trait Backend: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The OS keychain through the `keyring` crate.
pub struct OsKeychain;

impl Backend for OsKeychain {
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account).and_then(|e| e.set_password(secret)).map_err(|e| kind(&e))
    }
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, account).and_then(|e| e.get_password()) {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(kind(&e)),
        }
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, account).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(kind(&e)),
        }
    }
}

/// Error kind only: keyring errors never contain the secret, but keep it short anyway.
fn kind(e: &keyring::Error) -> String {
    match e {
        keyring::Error::NoStorageAccess(_) => "no keychain access".into(),
        keyring::Error::PlatformFailure(_) => "keychain failure".into(),
        keyring::Error::NoEntry => "no entry".into(),
        _ => "keychain error".into(),
    }
}

pub struct KeyStore {
    /// The 0600 fallback file
    pub file: PathBuf,
    pub keychain: Option<Box<dyn Backend>>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileKeys {
    version: u32,
    keys: BTreeMap<String, String>,
}

impl KeyStore {
    /// The real store. `WB_NO_KEYCHAIN=1` skips the keychain (headless tests, locked-down machines).
    pub fn system(config_dir: &Path) -> KeyStore {
        let off = std::env::var("WB_NO_KEYCHAIN").map(|v| v == "1").unwrap_or(false);
        KeyStore { file: config_dir.join("manual-keys.json"), keychain: if off { None } else { Some(Box::new(OsKeychain)) } }
    }

    fn read_file(&self) -> FileKeys {
        std::fs::read_to_string(&self.file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn write_file(&self, f: &FileKeys) -> Result<(), AppError> {
        let mut out = FileKeys { version: 1, keys: f.keys.clone() };
        out.version = 1;
        settings::write_atomic(&self.file, (serde_json::to_string_pretty(&out).unwrap() + "\n").as_bytes())
    }

    /// Save a key. Tries the keychain and reads it back; falls back to the private file.
    pub fn save(&self, account: &str, secret: &str) -> Result<Store, AppError> {
        if let Some(k) = &self.keychain {
            if k.set(account, secret).is_ok() && k.get(account).ok().flatten().as_deref() == Some(secret) {
                // A copy left in the file from an earlier fallback must not linger.
                self.remove_from_file(account)?;
                return Ok(Store::Keychain);
            }
        }
        let mut f = self.read_file();
        f.keys.insert(account.into(), secret.into());
        self.write_file(&f)?;
        Ok(Store::File)
    }

    pub fn load(&self, account: &str, store: Store) -> Option<String> {
        match store {
            Store::Keychain => self.keychain.as_ref().and_then(|k| k.get(account).ok().flatten()),
            Store::File => self.read_file().keys.get(account).cloned(),
        }
        .filter(|s| !s.is_empty())
    }

    fn remove_from_file(&self, account: &str) -> Result<(), AppError> {
        let mut f = self.read_file();
        if f.keys.remove(account).is_some() {
            self.write_file(&f)?;
        }
        Ok(())
    }

    /// Remove a key from both places.
    pub fn delete(&self, account: &str) -> Result<(), AppError> {
        if let Some(k) = &self.keychain {
            let _ = k.delete(account);
        }
        self.remove_from_file(account)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory keychain; `broken` makes every call fail like a missing Secret Service.
    #[derive(Default)]
    pub struct FakeKeychain {
        pub broken: bool,
        pub items: Mutex<BTreeMap<String, String>>,
    }
    impl Backend for FakeKeychain {
        fn set(&self, a: &str, s: &str) -> Result<(), String> {
            if self.broken { return Err("no keychain access".into()); }
            self.items.lock().unwrap().insert(a.into(), s.into());
            Ok(())
        }
        fn get(&self, a: &str) -> Result<Option<String>, String> {
            if self.broken { return Err("no keychain access".into()); }
            Ok(self.items.lock().unwrap().get(a).cloned())
        }
        fn delete(&self, a: &str) -> Result<(), String> {
            self.items.lock().unwrap().remove(a);
            Ok(())
        }
    }

    const KEY: &str = "sk-relay-0123456789abcdefXYZ";

    #[test]
    fn keychain_first_and_nothing_in_the_file() {
        let d = tempfile::tempdir().unwrap();
        let ks = KeyStore { file: d.path().join("manual-keys.json"), keychain: Some(Box::new(FakeKeychain::default())) };
        assert_eq!(ks.save("manual:a", KEY).unwrap(), Store::Keychain);
        assert!(!ks.file.exists());
        assert_eq!(ks.load("manual:a", Store::Keychain).as_deref(), Some(KEY));
        ks.delete("manual:a").unwrap();
        assert_eq!(ks.load("manual:a", Store::Keychain), None);
    }

    #[test]
    fn falls_back_to_a_private_file_when_the_keychain_is_unavailable() {
        let d = tempfile::tempdir().unwrap();
        for keychain in [None, Some(Box::new(FakeKeychain { broken: true, ..Default::default() }) as Box<dyn Backend>)] {
            let ks = KeyStore { file: d.path().join("sub").join("manual-keys.json"), keychain };
            assert_eq!(ks.save("manual:b", KEY).unwrap(), Store::File);
            assert_eq!(ks.load("manual:b", Store::File).as_deref(), Some(KEY));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(std::fs::metadata(&ks.file).unwrap().permissions().mode() & 0o777, 0o600);
            }
            ks.delete("manual:b").unwrap();
            assert_eq!(ks.load("manual:b", Store::File), None);
            assert!(!std::fs::read_to_string(&ks.file).unwrap().contains(KEY));
        }
    }

    #[test]
    fn moving_to_the_keychain_removes_the_file_copy() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("manual-keys.json");
        KeyStore { file: file.clone(), keychain: None }.save("manual:c", KEY).unwrap();
        let ks = KeyStore { file: file.clone(), keychain: Some(Box::new(FakeKeychain::default())) };
        assert_eq!(ks.save("manual:c", KEY).unwrap(), Store::Keychain);
        assert!(!std::fs::read_to_string(&file).unwrap().contains(KEY));
    }
}
