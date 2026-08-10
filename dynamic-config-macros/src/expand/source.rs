//! Source expression generation: one `Source` per configured file.

use std::path::Path;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, LitStr, Result};

/// Builds one `Source` expression, choosing the format from the extension.
///
/// The format goes through a redirect macro in the facade rather than naming
/// `Format::Toml` directly: a proc-macro cannot see which features the facade
/// was built with, and the redirect turns a disabled feature into a compile
/// error that says which one to enable.
pub(super) fn source_expression(file: &LitStr) -> Result<TokenStream> {
    let path = file.value();

    if path.is_empty() {
        return Err(Error::new(
            file.span(),
            "config file path must not be empty",
        ));
    }

    // `secrets.json.age` is JSON that happens to be encrypted, so the format
    // comes from the extension *under* the suffix and the source is a different
    // kind. A bare `secrets.age` names no format and is rejected below as an
    // unsupported `.age` extension, with a message that spells out the
    // `.json.age` shape it should have had.
    let encrypted = path
        .rsplit_once('.')
        .is_some_and(|(inner, suffix)| suffix.eq_ignore_ascii_case("age") && inner.contains('.'));

    let named = if encrypted {
        path.rsplit_once('.')
            .map_or(path.clone(), |(inner, _)| inner.to_owned())
    } else {
        path.clone()
    };

    let extension = Path::new(&named)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    let format = match extension.as_deref() {
        Some("json") => quote!(::dynamic_config::__format_json!()),
        Some("toml") => quote!(::dynamic_config::__format_toml!()),
        Some("yaml" | "yml") => quote!(::dynamic_config::__format_yaml!()),

        Some(extension) => {
            return Err(Error::new(
                file.span(),
                format!(
                    "unsupported config extension `.{extension}`; \
                     supported: `.json`, `.toml`, `.yaml`, `.yml`, \
                     each optionally with `.age` for an encrypted file"
                ),
            ));
        }

        None => {
            return Err(Error::new(
                file.span(),
                "config file has no extension; the format is inferred from it",
            ));
        }
    };

    if encrypted {
        return Ok(quote!(
            ::dynamic_config::__source_encrypted!(#file, #format)
        ));
    }

    Ok(quote!(::dynamic_config::Source::file(#file, #format)))
}
