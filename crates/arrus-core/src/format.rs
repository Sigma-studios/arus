//! The encrypted container used by ArrangeUs for `.arrus` (one choreography) and
//! `.aubkp` (full backup) files.
//!
//! ```text
//! file  = IV (16 bytes) || AES-256-CBC(PKCS7(utf8(json)))
//! ```
//!
//! The key is a fixed 32-byte secret compiled into the ArrangeUs app. It is not part
//! of this (public) repository: callers load it with [`FormatKey::from_env_or_file`]
//! or pass it in with [`FormatKey::new`].

use aes::Aes256;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};

use crate::Error;
use crate::model::{Backup, Choreography};

/// Length of the IV that prefixes every file.
pub const IV_LEN: usize = 16;
const BLOCK: usize = 16;

/// Environment variable holding the key (32 ASCII characters).
pub const KEY_ENV: &str = "ARUS_FORMAT_KEY";
/// File read when the environment variable is not set (path relative to the
/// working directory, or set `ARUS_FORMAT_KEY_FILE`).
pub const KEY_FILE_ENV: &str = "ARUS_FORMAT_KEY_FILE";
pub const DEFAULT_KEY_FILE: &str = ".arrus-key";

/// The 32-byte AES-256 key.
#[derive(Clone)]
pub struct FormatKey([u8; 32]);

impl std::fmt::Debug for FormatKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FormatKey(..)")
    }
}

impl FormatKey {
    pub fn new(bytes: &[u8]) -> Result<Self, Error> {
        let arr: [u8; 32] = bytes.try_into().map_err(|_| Error::KeyLength(bytes.len()))?;
        Ok(Self(arr))
    }

    /// `ARUS_FORMAT_KEY`, else the file named by `ARUS_FORMAT_KEY_FILE`, else
    /// `.arrus-key` in the working directory or one of its parents.
    pub fn from_env_or_file() -> Result<Self, Error> {
        if let Ok(v) = std::env::var(KEY_ENV) {
            return Self::new(v.trim_end_matches(['\n', '\r']).as_bytes());
        }
        let candidates: Vec<std::path::PathBuf> = match std::env::var(KEY_FILE_ENV) {
            Ok(p) => vec![p.into()],
            Err(_) => {
                let mut v = Vec::new();
                if let Ok(mut dir) = std::env::current_dir() {
                    loop {
                        v.push(dir.join(DEFAULT_KEY_FILE));
                        if !dir.pop() {
                            break;
                        }
                    }
                }
                v
            }
        };
        for path in candidates {
            if let Ok(bytes) = std::fs::read(&path) {
                let s = String::from_utf8_lossy(&bytes);
                return Self::new(s.trim_end_matches(['\n', '\r']).as_bytes());
            }
        }
        Err(Error::NoKey)
    }
}

/// Decrypt raw file bytes into the JSON text they contain.
pub fn decrypt(key: &FormatKey, blob: &[u8]) -> Result<String, Error> {
    if blob.len() < IV_LEN + BLOCK || !(blob.len() - IV_LEN).is_multiple_of(BLOCK) {
        return Err(Error::NotArrus);
    }
    let (iv, ct) = blob.split_at(IV_LEN);
    let dec = cbc::Decryptor::<Aes256>::new(key.0[..].into(), iv.into());
    let pt = dec.decrypt_padded_vec_mut::<Pkcs7>(ct).map_err(|_| Error::NotArrus)?;
    String::from_utf8(pt).map_err(|_| Error::NotArrus)
}

/// Encrypt JSON text into file bytes, with the given IV (use 16 random bytes).
pub fn encrypt(key: &FormatKey, json: &str, iv: [u8; IV_LEN]) -> Vec<u8> {
    let enc = cbc::Encryptor::<Aes256>::new(key.0[..].into(), (&iv).into());
    let ct = enc.encrypt_padded_vec_mut::<Pkcs7>(json.as_bytes());
    let mut out = Vec::with_capacity(IV_LEN + ct.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ct);
    out
}

/// What an imported file turned out to contain.
#[derive(Debug, Clone)]
pub enum Imported {
    Choreography(Box<Choreography>),
    Backup(Box<Backup>),
}

impl Imported {
    /// Every choreography in the file.
    pub fn into_choreographies(self) -> Vec<Choreography> {
        match self {
            Imported::Choreography(c) => vec![*c],
            Imported::Backup(b) => b.choreographies,
        }
    }
}

/// Decrypt and parse a `.arrus` or `.aubkp` file (detected from its content).
pub fn read(key: &FormatKey, blob: &[u8]) -> Result<Imported, Error> {
    let json = decrypt(key, blob)?;
    parse_json(&json)
}

/// Parse decrypted JSON: a backup has a `choreographies` array, a choreography has
/// `formations`.
pub fn parse_json(json: &str) -> Result<Imported, Error> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    if value.get("choreographies").is_some_and(|c| c.is_array()) {
        Ok(Imported::Backup(Box::new(serde_json::from_value(value)?)))
    } else {
        Ok(Imported::Choreography(Box::new(serde_json::from_value(value)?)))
    }
}

/// Serialize a choreography the way the app does (compact JSON) and encrypt it.
pub fn write_choreography(key: &FormatKey, c: &Choreography, iv: [u8; IV_LEN]) -> Result<Vec<u8>, Error> {
    Ok(encrypt(key, &serde_json::to_string(c)?, iv))
}
