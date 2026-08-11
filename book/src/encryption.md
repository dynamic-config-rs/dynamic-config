# Encrypted config files

A `secrets.json` in a repository is a problem everyone recognises. Encrypt it
with [`age`](https://age-encryption.org) and it decrypts at load time:

```text
config.toml            plain, in the repository
secrets.json.age       ciphertext, in the repository
```

```rust
#[dynamic_config]
#[derive(Deserialize)]
struct DbConfig {
    host: String,
    #[config(secret)]
    password: String,
}

// Once, before anything loads. A key is a process-wide fact, so this is a
// process-wide setting.
dynamic_config::set_decryptor(dynamic_config::age::Age::from_environment()?)?;

DbConfig::builder("db")
    .file("config.toml")
    .file("secrets.json.age")
    .init()?;
```

The `.age` suffix marks the file as encrypted; the extension **under** it says
what the plaintext is, so `secrets.json.age` is JSON. Everything else is
unchanged — same precedence, same profile variants
(`secrets.production.json.age`), watched the same way, skipped if it is not
there, and a value traced back to it names the file rather than "an inline
source".

The key comes from `SOPS_AGE_KEY_FILE`, `AGE_IDENTITY_FILE` or `AGE_SECRET_KEY`,
in that order — the SOPS variable first, because a machine set up for SOPS
already has it. `Age::from_identity_file`, `Age::from_key` and
`Age::from_passphrase` name one explicitly. Both binary and armored files are
read without being told which.

A file this key cannot open is an error naming the file, not a file quietly
skipped: a configuration that silently lost its secrets is worse than one that
refuses to start.

## What it does not do

**It does not keep secrets out of memory.** The resolved configuration holds
every value, because that is what configuration *is* — a program that can use a
password can read it. The decrypted text is zeroized once parsed, and
`#[config(secret)]` keeps values out of logs, but neither is a claim about
process memory.

**The cache is still plaintext.** [`save`](persistence.md#save) has an encrypting counterpart
— `save_encrypted`, taking the recipients at the call site, because *who may
read this file* is a decision about that write rather than a property of the
process. The [last-known-good cache](persistence.md#last-known-good) writes plaintext and says
so.

**It is not SOPS.** SOPS encrypts values in place and verifies a MAC over the
document — a format worth implementing properly or not at all. What is here
instead is the `Decryptor` trait: implement it, install it, and any scheme
works, including shelling out to `sops -d`.

```rust
impl Decryptor for MyScheme {
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error> { .. }
    fn describe(&self) -> String { "my-kms".to_owned() }
}
```
