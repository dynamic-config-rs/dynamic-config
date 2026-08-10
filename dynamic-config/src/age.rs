//! `age`-encrypted config files.
//!
//! [`age`] is a small, modern file-encryption format: one binary or armored
//! blob per file, recipients in the header, no key management to speak of. It
//! is what `secrets.json.age` in a repository is encrypted with, and this is the
//! [`Decryptor`] that reads it.
//!
//! ```sh
//! age-keygen -o key.txt
//! age -r "$(grep 'public key' key.txt | cut -d: -f2 | tr -d ' ')" \
//!     -o secrets.json.age secrets.json
//! rm secrets.json
//! ```
//!
//! ```rust,no_run
//! // The key comes from the environment, so it is not in the source or the
//! // repository. `SOPS_AGE_KEY_FILE` is honoured too, because a machine that
//! // already has one set should not need a second.
//! dynamic_config::set_decryptor(dynamic_config::age::Age::from_environment()?).ok();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Both binary and armored (`-a`, the `-----BEGIN AGE ENCRYPTED FILE-----`
//! form) files are read, without being told which — a repository that switched
//! to armor for a readable diff should not need a code change.
//!
//! [`age`]: https://age-encryption.org

use std::io::{Read, Write};

use age::Identity;

use crate::decrypt::{Decryptor, Encryptor};
use crate::error::Error;

/// Where `sops` looks for an age key file. Honoured first, because a machine
/// set up for SOPS already has it.
const SOPS_KEY_FILE: &str = "SOPS_AGE_KEY_FILE";

/// Where a bare `age` setup usually keeps one.
const AGE_KEY_FILE: &str = "AGE_IDENTITY_FILE";

/// A literal key, for a deployment that injects it rather than mounting a file.
const AGE_KEY: &str = "AGE_SECRET_KEY";

/// Decrypts `age`-encrypted config files.
///
/// Holds one or more identities. A file is decrypted by whichever of them the
/// recipients match, so a key being rotated is a matter of listing both for a
/// while rather than a flag day.
pub struct Age {
    identities: Vec<Box<dyn Identity + Send + Sync>>,
    described: String,
}

impl Age {
    /// Reads identities from an `age` key file.
    ///
    /// The file `age-keygen` writes: one or more `AGE-SECRET-KEY-…` lines, with
    /// comments allowed.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, holds no identity, or holds something that
    /// is not one.
    pub fn from_identity_file(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let named = path.display().to_string();

        let file = age::IdentityFile::from_file(named.clone())
            .map_err(|error| Error::decrypt(format!("cannot read {named}: {error}")))?;

        let identities = file.into_identities().map_err(|error| {
            Error::decrypt(format!("{named} holds no usable identity: {error}"))
        })?;

        Self::from_identities(identities, format!("age, keys from {named}"))
    }

    /// Parses identities from text, for a key that arrives in a variable rather
    /// than a file.
    ///
    /// # Errors
    ///
    /// If the text holds no identity, or holds something that is not one.
    pub fn from_key(text: &str) -> Result<Self, Error> {
        let file = age::IdentityFile::from_buffer(std::io::BufReader::new(text.as_bytes()))
            .map_err(|error| Error::decrypt(format!("the key is not an age identity: {error}")))?;

        let identities = file
            .into_identities()
            .map_err(|error| Error::decrypt(format!("the key is not an age identity: {error}")))?;

        Self::from_identities(identities, "age, key from the environment".to_owned())
    }

    /// Reads a key from the environment.
    ///
    /// In order: `SOPS_AGE_KEY_FILE`, `AGE_IDENTITY_FILE`, `AGE_SECRET_KEY`.
    /// The SOPS variable comes first because a machine set up for SOPS already
    /// has it, and asking for a second one that means the same thing is how
    /// deployments end up with two.
    ///
    /// # Errors
    ///
    /// If none of them is set, or the one that is cannot be read.
    pub fn from_environment() -> Result<Self, Error> {
        for variable in [SOPS_KEY_FILE, AGE_KEY_FILE] {
            if let Ok(path) = std::env::var(variable) {
                if !path.is_empty() {
                    return Self::from_identity_file(path);
                }
            }
        }

        if let Ok(key) = std::env::var(AGE_KEY) {
            if !key.is_empty() {
                return Self::from_key(&key);
            }
        }

        Err(Error::decrypt(format!(
            "no age key in the environment; set {SOPS_KEY_FILE}, {AGE_KEY_FILE} or {AGE_KEY}"
        )))
    }

    /// Decrypts with a passphrase.
    ///
    /// The `age -p` form. Weaker than a key file and deliberately awkward to
    /// automate — a passphrase a program can read is a passphrase in an
    /// environment variable — but it is what a hand-encrypted file uses.
    #[must_use]
    pub fn from_passphrase(passphrase: impl Into<String>) -> Self {
        let identity =
            age::scrypt::Identity::new(age::secrecy::SecretString::from(passphrase.into()));

        Self {
            identities: vec![Box::new(identity)],
            described: "age, passphrase".to_owned(),
        }
    }

    fn from_identities(
        identities: Vec<Box<dyn Identity + Send + Sync>>,
        described: String,
    ) -> Result<Self, Error> {
        if identities.is_empty() {
            return Err(Error::decrypt(format!("{described}: no identities")));
        }

        Ok(Self {
            identities,
            described,
        })
    }
}

impl Decryptor for Age {
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        // Armored input is detected rather than configured: a repository that
        // switched to `age -a` for a readable diff should not also need a code
        // change.
        let reader = age::armor::ArmoredReader::new(ciphertext);

        let decryptor = age::Decryptor::new_buffered(reader)
            .map_err(|error| Error::decrypt(format!("not a usable age file: {error}")))?;

        let mut plaintext = decryptor
            .decrypt(
                self.identities
                    .iter()
                    .map(|identity| identity.as_ref() as _),
            )
            .map_err(|error| match error {
                // Two ways of saying the same actionable thing: a key file that
                // is not a recipient gives `NoMatchingKeys`, and a passphrase
                // that is wrong gives `DecryptionFailed`. "Decryption failed"
                // on its own tells nobody what to do about it.
                age::DecryptError::NoMatchingKeys | age::DecryptError::DecryptionFailed => {
                    Error::decrypt(
                        "none of the configured identities is a recipient of this file; \
                         wrong key, or wrong passphrase",
                    )
                }
                error => Error::decrypt(error.to_string()),
            })?;

        let mut bytes = Vec::new();

        plaintext
            .read_to_end(&mut bytes)
            .map_err(|error| Error::decrypt(format!("the payload is damaged: {error}")))?;

        Ok(bytes)
    }

    fn describe(&self) -> String {
        self.described.clone()
    }
}

/// Who may read a file this program encrypts.
///
/// The other half of [`Age`]. Recipients are public keys, so this holds no
/// secret and its `Debug` says how many there are rather than which.
///
/// ```no_run
/// use dynamic_config::age::Recipients;
///
/// // The `age1…` line `age-keygen` prints, or a `recipients.txt` of them.
/// let recipients = Recipients::from_public_keys(["age1ql3z7hjy54pw3hyww5ayyfg7zqgvc7w3j2elw8zmrj2kg5sfn9aqmcac8p"])?;
/// # Ok::<(), dynamic_config::Error>(())
/// ```
pub struct Recipients {
    recipients: Vec<Box<dyn age::Recipient + Send + Sync>>,
    described: String,
}

impl Recipients {
    /// Parses `age1…` public keys.
    ///
    /// Several because a file usually has to be readable by more than one
    /// party — a deployment key and the operator who has to fix it at three in
    /// the morning.
    ///
    /// # Errors
    ///
    /// If any of them is not an age public key, or the list is empty: a file
    /// encrypted to nobody cannot be read by anybody.
    pub fn from_public_keys<I, S>(keys: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut recipients: Vec<Box<dyn age::Recipient + Send + Sync>> = Vec::new();

        for key in keys {
            let key = key.as_ref().trim();

            // A `recipients.txt` is comments and blank lines as well as keys.
            if key.is_empty() || key.starts_with('#') {
                continue;
            }

            let parsed: age::x25519::Recipient = key.parse().map_err(|error| {
                Error::decrypt(format!("`{key}` is not an age recipient: {error}"))
            })?;

            recipients.push(Box::new(parsed));
        }

        if recipients.is_empty() {
            return Err(Error::decrypt(
                "no recipients; a file encrypted to nobody cannot be read by anybody",
            ));
        }

        let described = format!("age, {} recipient(s)", recipients.len());

        Ok(Self {
            recipients,
            described,
        })
    }

    /// Reads recipients from a file, one `age1…` key per line.
    ///
    /// The `recipients.txt` shape `age -R` takes. Comments and blank lines are
    /// skipped.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, or holds something that is not a recipient.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let path = path.as_ref();

        let text = std::fs::read_to_string(path)
            .map_err(|error| Error::decrypt(format!("cannot read {}: {error}", path.display())))?;

        let mut recipients = Self::from_public_keys(text.lines())?;
        recipients.described = format!("age, recipients from {}", path.display());

        Ok(recipients)
    }

    /// Encrypts to a passphrase instead of a key.
    ///
    /// Matches [`Age::from_passphrase`] on the reading side. Weaker than a key
    /// and awkward to automate, which is the point: a passphrase a program can
    /// read is a passphrase in an environment variable.
    #[must_use]
    pub fn from_passphrase(passphrase: impl Into<String>) -> Self {
        let recipient =
            age::scrypt::Recipient::new(age::secrecy::SecretString::from(passphrase.into()));

        Self {
            recipients: vec![Box::new(recipient)],
            described: "age, passphrase".to_owned(),
        }
    }
}

impl Encryptor for Recipients {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let recipients = self
            .recipients
            .iter()
            .map(|recipient| recipient.as_ref() as &dyn age::Recipient);

        // `age::encrypt` takes a single recipient; the multi-recipient path
        // goes through the protocol type, which is also where the streaming
        // writer lives.
        let encryptor = age::Encryptor::with_recipients(recipients)
            .map_err(|error| Error::decrypt(format!("encrypting failed: {error}")))?;

        let mut ciphertext = Vec::new();

        let mut writer = encryptor
            .wrap_output(&mut ciphertext)
            .map_err(|error| Error::decrypt(format!("encrypting failed: {error}")))?;

        writer
            .write_all(plaintext)
            .map_err(|error| Error::decrypt(format!("encrypting failed: {error}")))?;

        // Not `flush`: the age writer needs `finish` to emit the final chunk,
        // and a file missing it decrypts as truncated rather than failing.
        writer
            .finish()
            .map_err(|error| Error::decrypt(format!("encrypting failed: {error}")))?;

        Ok(ciphertext)
    }

    fn describe(&self) -> String {
        self.described.clone()
    }
}

impl std::fmt::Debug for Recipients {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recipients")
            .field("recipients", &self.recipients.len())
            .field("from", &self.described)
            .finish()
    }
}

impl std::fmt::Debug for Age {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Age")
            .field("identities", &self.identities.len())
            .field("from", &self.described)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encrypts with a passphrase, so the tests have real ciphertext without a
    /// key file or a fixture that could rot.
    fn encrypt(plaintext: &[u8], passphrase: &str) -> Vec<u8> {
        let recipient =
            age::scrypt::Recipient::new(age::secrecy::SecretString::from(passphrase.to_owned()));

        age::encrypt(&recipient, plaintext).expect("encrypting should succeed")
    }

    #[test]
    fn a_file_encrypted_to_a_passphrase_comes_back() {
        let ciphertext = encrypt(br#"{"db": {"host": "localhost"}}"#, "hunter2");

        let plaintext = Age::from_passphrase("hunter2")
            .decrypt(&ciphertext)
            .expect("the passphrase matches");

        assert_eq!(plaintext, br#"{"db": {"host": "localhost"}}"#);
    }

    #[test]
    fn the_wrong_passphrase_says_so_rather_than_returning_rubbish() {
        let ciphertext = encrypt(b"{}", "hunter2");

        let error = Age::from_passphrase("hunter3")
            .decrypt(&ciphertext)
            .expect_err("the passphrase does not match");

        assert_eq!(error.kind(), crate::ErrorKind::Decrypt);
        assert!(error.to_string().contains("recipient"), "{error}");
    }

    #[test]
    fn something_that_is_not_an_age_file_is_a_clear_error() {
        let error = Age::from_passphrase("hunter2")
            .decrypt(br#"{"db": {"host": "localhost"}}"#)
            .expect_err("plaintext is not an age file");

        assert!(
            error.to_string().contains("not a usable age file"),
            "{error}"
        );
    }

    #[test]
    fn an_armored_file_is_read_without_being_told() {
        let recipient =
            age::scrypt::Recipient::new(age::secrecy::SecretString::from("hunter2".to_owned()));
        let armored =
            age::encrypt_and_armor(&recipient, b"{\"db\": {}}").expect("armoring should succeed");

        assert!(
            armored.starts_with("-----BEGIN AGE ENCRYPTED FILE-----"),
            "{armored}"
        );

        let plaintext = Age::from_passphrase("hunter2")
            .decrypt(armored.as_bytes())
            .expect("armor is detected, not configured");

        assert_eq!(plaintext, b"{\"db\": {}}");
    }

    #[test]
    fn an_identity_file_is_read_from_disk() {
        let identity = age::x25519::Identity::generate();
        let key = identity.to_string();

        // A fresh directory per run: two test processes sharing a fixed name
        // under `temp_dir()` would race on the `remove_dir_all`.
        let directory = tempfile::tempdir().unwrap();

        let path = directory.path().join("key.txt");
        std::fs::write(
            &path,
            format!(
                "# a comment\n{}\n",
                age::secrecy::ExposeSecret::expose_secret(&key)
            ),
        )
        .unwrap();

        let age = Age::from_identity_file(&path).expect("the file holds one identity");

        assert!(age.describe().contains("key.txt"), "{}", age.describe());

        let ciphertext = age::encrypt(&identity.to_public(), b"{\"db\": {}}").unwrap();

        assert_eq!(age.decrypt(&ciphertext).unwrap(), b"{\"db\": {}}");
    }

    #[test]
    fn a_file_that_holds_no_identity_says_so() {
        let directory = tempfile::tempdir().unwrap();

        let path = directory.path().join("key.txt");
        std::fs::write(&path, "# nothing but a comment\n").unwrap();

        let error = Age::from_identity_file(&path).expect_err("there is no key in there");

        assert_eq!(error.kind(), crate::ErrorKind::Decrypt);
    }

    #[test]
    fn a_missing_identity_file_names_itself() {
        let error = Age::from_identity_file("/no/such/key.txt").expect_err("there is no such file");

        assert!(error.to_string().contains("/no/such/key.txt"), "{error}");
    }

    #[test]
    fn debug_never_prints_a_key() {
        let printed = format!("{:?}", Age::from_passphrase("hunter2"));

        assert!(!printed.contains("hunter2"), "{printed}");
    }
}
