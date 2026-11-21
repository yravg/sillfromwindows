//! OWNER: worker C1. Identity, saved Macs and preferences on disk.
//!
//! One folder per user: `%APPDATA%\Sill` on Windows, `~/.config/sill` on Linux, or `SILL_HOME`
//! (tests). Files: `identity.key` (the device's P-256 key as PKCS#8; wrapped with DPAPI on
//! Windows, a 0600 file elsewhere), `macs.json` (sill-net `SavedMac`s, the Mac's own JSON names),
//! `prefs.json` (this PC's preferences). Every write goes to a temporary file and is renamed, so a
//! crash never leaves half a file; a file that does not parse is kept as `*.bad` and read as empty
//! (a saved Mac is never silently lost to a bad byte, the identity never silently replaced).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sill_net::identity::{load_or_create, DeviceIdentity, IdentityError, KeyStore};
use sill_net::pair_client::SavedMac;

use crate::model::Preferences;

pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The user's folder (`SILL_HOME` first), created when missing.
    pub fn open() -> io::Result<Store> {
        let dir = match std::env::var_os("SILL_HOME") {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ => {
                let base = dirs::config_dir()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user folder"))?;
                base.join(if cfg!(windows) { "Sill" } else { "sill" })
            }
        };
        Store::at(dir)
    }

    pub fn at(dir: PathBuf) -> io::Result<Store> {
        fs::create_dir_all(&dir)?;
        Ok(Store { dir })
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The device identity: stored, or made once and read back (DeviceKeyPolicy).
    pub fn identity(&self) -> Result<DeviceIdentity, IdentityError> {
        load_or_create(&FileKeyStore {
            path: self.dir.join("identity.key"),
        })
    }

    pub fn load_macs(&self) -> Vec<SavedMac> {
        read_json(&self.dir.join("macs.json")).unwrap_or_default()
    }

    pub fn save_macs(&self, macs: &[SavedMac]) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(macs).map_err(io::Error::other)?;
        write_atomic(&self.dir.join("macs.json"), &json, false)
    }

    pub fn load_prefs(&self) -> Preferences {
        read_json::<PrefsFile>(&self.dir.join("prefs.json"))
            .map(Into::into)
            .unwrap_or_default()
    }

    pub fn save_prefs(&self, prefs: &Preferences) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(&PrefsFile::from(prefs)).map_err(io::Error::other)?;
        write_atomic(&self.dir.join("prefs.json"), &json, false)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PrefsFile {
    mute: bool,
    share_clipboard: bool,
    reverse_scrolling: bool,
    windows_key_is_command: bool,
}

impl Default for PrefsFile {
    fn default() -> Self {
        Preferences::default().into_file()
    }
}

impl Preferences {
    fn into_file(self) -> PrefsFile {
        PrefsFile {
            mute: self.mute,
            share_clipboard: self.share_clipboard,
            reverse_scrolling: self.reverse_scrolling,
            windows_key_is_command: self.windows_key_is_command,
        }
    }
}

impl From<&Preferences> for PrefsFile {
    fn from(p: &Preferences) -> Self {
        p.clone().into_file()
    }
}

impl From<PrefsFile> for Preferences {
    fn from(p: PrefsFile) -> Self {
        Preferences {
            mute: p.mute,
            share_clipboard: p.share_clipboard,
            reverse_scrolling: p.reverse_scrolling,
            windows_key_is_command: p.windows_key_is_command,
        }
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(_) => {
            let _ = fs::rename(path, path.with_extension("json.bad"));
            None
        }
    }
}

/// Write `bytes` beside `path` and rename over it. `private` makes the file owner-only (Unix).
fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    fs::rename(&tmp, path)
}

/// The device key in a file: DPAPI-wrapped on Windows (this user, this PC), raw and 0600 elsewhere.
struct FileKeyStore {
    path: PathBuf,
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> Result<Option<Vec<u8>>, IdentityError> {
        match fs::read(&self.path) {
            Ok(stored) => unprotect(&stored).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(IdentityError::Store(e.to_string())),
        }
    }

    fn save(&self, pkcs8: &[u8]) -> Result<(), IdentityError> {
        let wrapped = protect(pkcs8)?;
        write_atomic(&self.path, &wrapped, true).map_err(|e| IdentityError::Store(e.to_string()))
    }

    fn delete(&self) -> Result<(), IdentityError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(IdentityError::Store(e.to_string())),
        }
    }
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>, IdentityError> {
    dpapi(plain, true)
}
#[cfg(windows)]
fn unprotect(stored: &[u8]) -> Result<Vec<u8>, IdentityError> {
    dpapi(stored, false)
}

/// CryptProtectData / CryptUnprotectData with the current user's key and no UI.
#[cfg(windows)]
fn dpapi(input: &[u8], protect: bool) -> Result<Vec<u8>, IdentityError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: the in-blob points at `input`, which outlives the call; `out` is filled by the
    // system, copied, and freed with LocalFree as the documentation says.
    unsafe {
        let r = if protect {
            CryptProtectData(
                &blob_in,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &blob_in,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        r.map_err(|e| IdentityError::Store(format!("DPAPI: {e}")))?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Ok(bytes)
    }
}

#[cfg(not(windows))]
fn protect(plain: &[u8]) -> Result<Vec<u8>, IdentityError> {
    Ok(plain.to_vec())
}
#[cfg(not(windows))]
fn unprotect(stored: &[u8]) -> Result<Vec<u8>, IdentityError> {
    Ok(stored.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sill-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn identity_is_made_once_and_read_back() {
        let s = Store::at(temp("id")).unwrap();
        let a = s.identity().unwrap();
        let b = s.identity().unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(s.dir().join("identity.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_stored_value_that_is_no_key_is_an_error_not_a_new_identity() {
        let s = Store::at(temp("badkey")).unwrap();
        fs::write(s.dir().join("identity.key"), b"nonsense").unwrap();
        assert!(s.identity().is_err());
        assert_eq!(fs::read(s.dir().join("identity.key")).unwrap(), b"nonsense");
    }

    fn mac(id: &str) -> SavedMac {
        SavedMac {
            name: "Studio".into(),
            mac_id: id.into(),
            fingerprint: "Tyl0TmZlUyPNKVJsJ_vePXGZMbcQgFBFZtA-06cZ4ik".into(),
            addresses: vec!["100.88.89.4".into()],
            port: 7455,
            recognition_key: Some("KHoqX07IiFA9K5TJgfumAldTpOyKR24EwHvtdvXqs94".into()),
            method: "code".into(),
        }
    }

    #[test]
    fn saved_macs_round_trip_and_a_bad_file_is_kept_aside() {
        let s = Store::at(temp("macs")).unwrap();
        assert!(s.load_macs().is_empty());
        s.save_macs(&[mac("A"), mac("B")]).unwrap();
        assert_eq!(s.load_macs(), vec![mac("A"), mac("B")]);
        fs::write(s.dir().join("macs.json"), b"{not json").unwrap();
        assert!(s.load_macs().is_empty());
        assert!(s.dir().join("macs.json.bad").exists());
    }

    #[test]
    fn preferences_default_then_persist() {
        let s = Store::at(temp("prefs")).unwrap();
        assert_eq!(s.load_prefs(), Preferences::default());
        let p = Preferences {
            mute: true,
            share_clipboard: false,
            reverse_scrolling: true,
            windows_key_is_command: true,
        };
        s.save_prefs(&p).unwrap();
        assert_eq!(s.load_prefs(), p);
        // a file from an older build with fewer keys still reads, the rest by default
        fs::write(s.dir().join("prefs.json"), br#"{"mute":true}"#).unwrap();
        let q = s.load_prefs();
        assert!(q.mute && q.share_clipboard);
    }
}
