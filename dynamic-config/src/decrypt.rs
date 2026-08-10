//! Config files that are encrypted on disk.
//!
//! A `secrets.json` in a repository is a problem everyone recognises. The usual
//! answer is to encrypt it — `age` is the modern one — and decrypt it wherever
//! it is used, which for a configuration file means at load time.
//!
//! ```text
//! config.toml            plain, in the repository
//! secrets.json.age       ciphertext, in the repository
//! ```
//!
//! ```rust,no_run
//! # #[cfg(feature = "age")] {
//! # use serde::Deserialize;
//! // Once, before anything loads. A key is a process-wide fact, so this is a
//! // process-wide setting.
//! dynamic_config::set_decryptor(
//!     dynamic_config::age::Age::from_environment()?,
//! )
//! .ok();
//!
//! #[dynamic_config::dynamic_config(files = ["config.toml", "secrets.json.age"], key = "db")]
//! #[derive(Deserialize)]
//! struct DbConfig {
//!     host: String,
//!     #[config(secret)]
//!     password: String,
//! }
//!
//! DbConfig::init()?;
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The `.age` suffix is what marks a file as encrypted; the extension *under*
//! it says what the plaintext is, so `secrets.json.age` is JSON. Everything
//! else is unchanged: it is a file like any other, in the same precedence
//! order, watched the same way, and a profile variant is
//! `secrets.production.json.age`.
//!
//! # What this does not do
//!
//! **It does not keep secrets out of memory.** The resolved configuration holds
//! every value, because that is what configuration *is* — a program that can
//! use a password can read it. The decrypted text is zeroized once parsed, and
//! `#[config(secret)]` keeps values out of logs, but neither is a claim about
//! process memory.
//!
//! **It does not encrypt anything.** `save` and the last-known-good cache both
//! write plaintext, and say so. Decryption is for the file the program *reads*.
//!
//! # Bringing your own scheme
//!
//! [`Decryptor`] is behind the `decrypt` feature, which `age` turns on for you.
//! Enable it directly for a scheme of your own — SOPS through `sops -d`, a KMS,
//! anything — and everything else stays as it is, including the wiping.
//!
//! **It is not SOPS.** SOPS encrypts values in place, leaving the keys
//! readable, and verifies a MAC over the whole document — a format worth
//! implementing properly or not at all. What is here instead is
//! [`Decryptor`]: implement it, install it, and any scheme works, including
//! shelling out to `sops -d`.

use std::sync::OnceLock;

use crate::error::{Error, ErrorKind};

/// Turns the bytes of an encrypted config file into configuration text.
///
/// One implementation ships — [`age::Age`](crate::age::Age) — and this trait is
/// why there could be others. A program using SOPS, a KMS, or an internal
/// scheme implements this and installs it; nothing else in the crate changes.
pub trait Decryptor: Send + Sync + 'static {
    /// Decrypts `ciphertext` into configuration text.
    ///
    /// The returned bytes are parsed as UTF-8 and zeroized afterwards.
    ///
    /// # Errors
    ///
    /// Whatever going wrong looks like for this scheme. Use
    /// [`Error::decrypt`] so the failure is categorised consistently.
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error>;

    /// How to name this decryptor in an error.
    fn describe(&self) -> String;
}

/// Turns configuration text into the bytes of an encrypted file.
///
/// The other direction from [`Decryptor`], and deliberately *not* installed
/// process-wide: who may read a file this program writes is a decision about
/// that write, not a property of the process. It is passed to `save_encrypted`
/// at the call site, where somebody can see which recipients they chose.
pub trait Encryptor: Send + Sync {
    /// Encrypts `plaintext` into the bytes to write.
    ///
    /// # Errors
    ///
    /// Whatever going wrong looks like for this scheme. Use
    /// [`Error::decrypt`](crate::Error::decrypt), which covers both directions.
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error>;

    /// How to name this encryptor in an error.
    fn describe(&self) -> String;
}

static DECRYPTOR: OnceLock<Box<dyn Decryptor>> = OnceLock::new();

/// Installs the decryptor, once per process.
///
/// Process-wide rather than per config type, because a decryption key is a
/// process-wide fact: the program either can read its own secrets or cannot.
/// Two config types needing two different keys is a shape this deliberately
/// does not model.
///
/// Call it before the first `init()`. An encrypted file loaded without one
/// fails with an error saying so, rather than being silently skipped — a
/// configuration that quietly lost its secrets is worse than one that refuses
/// to start.
///
/// # Errors
///
/// If one is already installed. The rejected decryptor is returned rather than
/// dropped, so a caller can tell "already set" from "failed".
pub fn set_decryptor(decryptor: impl Decryptor) -> Result<(), Box<dyn Decryptor>> {
    // `OnceLock::set` wants the error type to be `Debug`; a trait object is not,
    // and requiring `Debug` of every decryptor to satisfy a `Result` would be
    // the tail wagging the dog.
    match DECRYPTOR.set(Box::new(decryptor)) {
        Ok(()) => Ok(()),
        Err(rejected) => Err(rejected),
    }
}

/// Whether a decryptor is installed.
pub fn has_decryptor() -> bool {
    DECRYPTOR.get().is_some()
}

/// Decrypts `ciphertext` with the installed decryptor.
///
/// `path` only names the file in errors.
pub(crate) fn decrypt(ciphertext: &[u8], path: &str) -> Result<Plaintext, Error> {
    let Some(decryptor) = DECRYPTOR.get() else {
        return Err(Error::new(
            ErrorKind::Decrypt,
            format!(
                "{path} is encrypted and no decryptor is installed; call \
                 `dynamic_config::set_decryptor` before loading"
            ),
        ));
    };

    let plaintext = decryptor
        .decrypt(ciphertext)
        .map_err(|error| error.prepend_key(format!("{path} ({})", decryptor.describe())))?;

    Plaintext::new(plaintext, path)
}

/// Decrypted configuration text, wiped when it goes out of scope.
///
/// Defence in depth rather than a guarantee: the values end up in the resolved
/// configuration regardless, because that is what configuration is. What this
/// buys is that the *whole file* — including anything the struct does not read —
/// does not sit in a buffer for the life of the process.
pub(crate) struct Plaintext {
    text: String,
}

impl Plaintext {
    fn new(bytes: Vec<u8>, path: &str) -> Result<Self, Error> {
        match String::from_utf8(bytes) {
            Ok(text) => Ok(Self { text }),
            Err(error) => {
                let rendered = error.utf8_error().to_string();

                // The failure path wipes too. `FromUtf8Error` hands the bytes
                // back and would otherwise drop them unwiped — and decrypted
                // secrets sitting in freed memory is the exact thing this type
                // exists to prevent.
                let mut bytes = error.into_bytes();

                {
                    use zeroize::Zeroize;

                    bytes.zeroize();
                }

                Err(Error::new(
                    ErrorKind::Decrypt,
                    format!("{path}: the decrypted bytes are not UTF-8: {rendered}"),
                ))
            }
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

impl Drop for Plaintext {
    fn drop(&mut self) {
        use zeroize::Zeroize;

        self.text.zeroize();
    }
}

impl std::fmt::Debug for Plaintext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the contents: this type exists because they are secret.
        f.debug_struct("Plaintext")
            .field("bytes", &self.text.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_never_prints_what_it_holds() {
        let plaintext = Plaintext::new(b"{\"password\": \"hunter2\"}".to_vec(), "x").unwrap();

        let printed = format!("{plaintext:?}");

        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(
            printed.contains("23"),
            "the length is fine to show: {printed}"
        );
    }

    #[test]
    fn bytes_that_are_not_text_name_the_file() {
        let error = Plaintext::new(vec![0xff, 0xfe], "secrets.json.age").unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Decrypt);
        assert!(error.to_string().contains("secrets.json.age"), "{error}");
    }

    #[test]
    fn an_encrypted_file_with_no_decryptor_says_what_to_call() {
        // The static is process-wide and other tests may install one, so this
        // asserts on the message rather than on the absence.
        if has_decryptor() {
            return;
        }

        let error = decrypt(b"whatever", "secrets.json.age").unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Decrypt);
        assert!(error.to_string().contains("set_decryptor"), "{error}");
    }
}
