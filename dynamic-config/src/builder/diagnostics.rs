//! Questions a builder answers without installing anything.
//!
//! Every method here is a read: `explain`, `source_of`, `is_set`,
//! `snapshot`, `check` — and, with the `schema` feature, the JSON Schema
//! for the file this section lives in. All of them run through the same
//! `with_spec` funnel the loads use, so an answer cannot drift from what a
//! load would do.

use serde::de::DeserializeOwned;

use crate::error::Error;

use super::Builder;

impl<T: DeserializeOwned> Builder<T> {
    /// Explains `path` against this builder's sources; see [`crate::explain`].
    ///
    /// A builder that knows which fields are secret — every generated
    /// `builder()` does — hands back a path under one of them already
    /// redacted, the same as the type-level `explain`. A bare
    /// [`Builder::new`] knows no secrets and redacts nothing; pass the
    /// result through [`Explanation::redacted`](crate::Explanation::redacted)
    /// for a path you know to be sensitive.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn explain(&self, path: &str) -> Result<crate::Explanation, Error> {
        let explanation = self.with_spec(|spec| crate::explain::explain(spec, path))?;

        // The same head-of-path check as the generated method: secrets are
        // field names, and every path under one is the secret's.
        if let Some(secrets) = &self.secrets {
            let head = path.split('.').next().unwrap_or(path);

            if secrets.iter().any(|secret| secret == head) {
                return Ok(explanation.redacted());
            }
        }

        Ok(explanation)
    }

    /// Where the value at `path` would come from, if anything supplies it.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn source_of(&self, path: &str) -> Result<Option<crate::Origin>, Error> {
        self.with_spec(|spec| crate::loader::source_of(spec, path))
    }

    /// Whether anything supplies `path`.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn is_set(&self, path: &str) -> Result<bool, Error> {
        self.with_spec(|spec| crate::loader::is_set(spec, path))
    }

    /// Resolves the section without deserializing it.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn snapshot(&self) -> Result<crate::Snapshot, Error> {
        self.with_spec(crate::loader::snapshot)
    }

    /// What this configuration resolves to, and whether it would load —
    /// see [`check`](crate::check). Unknown-key detection uses the field
    /// names only the generated `builder()` carries; a bare builder reports
    /// none.
    ///
    /// # Errors
    ///
    /// Only if the sources cannot be read at all.
    pub fn check(&self) -> Result<crate::Report, Error> {
        self.with_spec(|spec| crate::check::<T>(spec, self.fields))
    }
}

#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
impl<T: DeserializeOwned> Builder<T> {
    /// A JSON Schema for the *file* this section lives in.
    ///
    /// The struct's schema wrapped under this builder's key, with
    /// `#[config(secret)]` fields carrying `writeOnly` — which the generated
    /// `builder()` knows and a bare one does not. Combine several with
    /// [`schema::merge`](crate::schema::merge) when more than one config
    /// type shares a file.
    #[must_use]
    pub fn schema(&self) -> serde_json::Value
    where
        T: schemars::JsonSchema,
    {
        let secrets = self.secrets.clone().unwrap_or_default();
        let secret_refs: Vec<&str> = secrets.iter().map(String::as_str).collect();

        crate::schema::section(&self.key, schemars::schema_for!(T).into(), &secret_refs)
    }
}
