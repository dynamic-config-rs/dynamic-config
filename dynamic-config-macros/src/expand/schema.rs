//! Schema and field-attribute handling: the `#[config(..)]` options, serde
//! rename resolution, and the opt-in JSON schema surface.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Fields, Ident, ItemStruct, LitStr, Result};

/// Field attribute this macro consumes, e.g. `#[config(secret)]`.
const FIELD_ATTRIBUTE: &str = "config";

/// The schema methods, when `schema` was asked for.
///
/// Opt-in like `save`, and for the same reason: the method needs a trait the
/// user has to derive. A `where Self: JsonSchema` clause cannot express that
/// — rustc rejects an inherent method whose bound a concrete `Self` does not
/// meet, at the definition rather than at the call.
pub(super) fn schema_methods(schema: bool, key: &LitStr, secret_names: &[String]) -> TokenStream {
    if schema {
        quote! {
            ::dynamic_config::__schema_methods!(#key, &[#(#secret_names),*]);
        }
    } else {
        quote! {}
    }
}

/// The field names a configuration section may legitimately carry.
///
/// Follows `#[serde(rename = "..")]`, since that is the name the file uses.
/// Returns `None` when any field is `#[serde(flatten)]`: a flattened field
/// absorbs keys the outer struct never names, so reporting those as typos would
/// be worse than reporting nothing.
pub(super) fn field_names(input: &ItemStruct) -> Option<Vec<String>> {
    let Fields::Named(fields) = &input.fields else {
        return None;
    };

    let mut names = Vec::new();

    for field in &fields.named {
        let mut renamed = None;
        let mut aliases = Vec::new();
        let mut flattened = false;

        for attribute in &field.attrs {
            if !attribute.path().is_ident("serde") {
                continue;
            }

            // Best effort: serde's attribute grammar is larger than this, and
            // anything unrecognised simply leaves the field under its own name.
            let _ = attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("flatten") {
                    flattened = true;
                }

                if meta.path.is_ident("alias") {
                    // An alias is a name the file may legitimately use, so it
                    // must not be reported as an unknown key.
                    if let Ok(value) = meta.value() {
                        if let Ok(name) = value.parse::<LitStr>() {
                            aliases.push(name.value());
                        }
                    }
                }

                if meta.path.is_ident("rename") {
                    if let Ok(value) = meta.value() {
                        if let Ok(name) = value.parse::<LitStr>() {
                            renamed = Some(name.value());
                        }
                    }
                }

                Ok(())
            });
        }

        if flattened {
            return None;
        }

        names.push(renamed.unwrap_or_else(|| {
            field
                .ident
                .as_ref()
                .expect("named fields always have an identifier")
                .to_string()
        }));
        names.append(&mut aliases);
    }

    Some(names)
}

/// Reads and removes every `#[config(..)]` field attribute.
///
/// Returns the fields marked `secret`, as `(rust_ident, serde_name)` pairs:
/// the ident drives the generated `Debug`, and the serde name — resolved
/// through `#[serde(rename)]` and the container's `rename_all` — is what the
/// resolved configuration tree actually uses. Redacting by the Rust name
/// used to write a "redacted" cache that still contained every renamed
/// secret, in the clear.
pub(super) fn take_field_options(input: &mut ItemStruct) -> Result<Vec<(Ident, String)>> {
    let rename_all = container_rename_all(input);

    let Fields::Named(fields) = &mut input.fields else {
        // Nothing to read, and nothing that could have been marked. A tuple or
        // unit struct is a perfectly good config type otherwise.
        return Ok(Vec::new());
    };

    let mut secrets = Vec::new();

    for field in &mut fields.named {
        let mut is_secret = false;
        let mut error = None;

        field.attrs.retain(|attribute| {
            if !attribute.path().is_ident(FIELD_ATTRIBUTE) {
                return true;
            }

            let parsed = attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("secret") {
                    is_secret = true;

                    return Ok(());
                }

                Err(meta.error("unknown option; the only one is `secret`"))
            });

            if let Err(parse_error) = parsed {
                error.get_or_insert(parse_error);
            }

            false
        });

        if let Some(error) = error {
            return Err(error);
        }

        if is_secret {
            let ident = field
                .ident
                .clone()
                .expect("named fields always have an identifier");

            let serde_name = field_rename(field)
                .or_else(|| {
                    rename_all
                        .as_deref()
                        .map(|rule| apply_rename_all(rule, &ident.to_string()))
                })
                .unwrap_or_else(|| ident.to_string());

            secrets.push((ident, serde_name));
        }
    }

    Ok(secrets)
}

/// The container-level `#[serde(rename_all = "...")]` rule, if any.
fn container_rename_all(input: &ItemStruct) -> Option<String> {
    for attribute in &input.attrs {
        if !attribute.path().is_ident("serde") {
            continue;
        }

        let mut found = None;

        let _ = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                if let Ok(value) = meta.value() {
                    if let Ok(rule) = value.parse::<LitStr>() {
                        found = Some(rule.value());
                    }
                }
            }

            Ok(())
        });

        if found.is_some() {
            return found;
        }
    }

    None
}

/// The field-level `#[serde(rename = "...")]`, if any.
fn field_rename(field: &syn::Field) -> Option<String> {
    for attribute in &field.attrs {
        if !attribute.path().is_ident("serde") {
            continue;
        }

        let mut found = None;

        let _ = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                if let Ok(value) = meta.value() {
                    if let Ok(name) = value.parse::<LitStr>() {
                        found = Some(name.value());
                    }
                }
            }

            Ok(())
        });

        if found.is_some() {
            return found;
        }
    }

    None
}

/// serde's `rename_all` conventions, applied to a snake_case Rust ident.
///
/// The full serde set; an unrecognised rule leaves the name alone, which is
/// also what serde does with a rule it does not know.
///
/// serde applies these to *fields* without restructuring: `lowercase` and
/// `UPPERCASE` keep every underscore (on a snake_case ident, `lowercase` is
/// the identity and `UPPERCASE` equals `SCREAMING_SNAKE_CASE`). Only the
/// Pascal/camel/kebab families reshape the name.
fn apply_rename_all(rule: &str, name: &str) -> String {
    let words: Vec<&str> = name.split('_').filter(|word| !word.is_empty()).collect();

    let capitalize = |word: &str| {
        let mut chars = word.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().collect::<String>() + chars.as_str()
        })
    };

    match rule {
        "lowercase" => name.to_lowercase(),
        "UPPERCASE" => name.to_uppercase(),
        "PascalCase" => words.iter().map(|word| capitalize(word)).collect(),
        "camelCase" => {
            let mut out = String::new();
            for (index, word) in words.iter().enumerate() {
                if index == 0 {
                    out.push_str(word);
                } else {
                    out.push_str(&capitalize(word));
                }
            }
            out
        }
        "SCREAMING_SNAKE_CASE" => name.to_uppercase(),
        "kebab-case" => name.replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => name.to_uppercase().replace('_', "-"),
        _ => name.to_owned(),
    }
}
