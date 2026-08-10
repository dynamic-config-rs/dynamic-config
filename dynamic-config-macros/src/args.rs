//! Parsing and validation of the `#[dynamic_config(..)]` argument list.
//!
//! Every rejection here is a compile error pointing at the offending token,
//! because a configuration mistake caught by `rustc` is one that never reaches
//! a deployment.

use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Error, Ident, LitInt, LitStr, Result, Token};

/// Debounce window used when `debounce` is not given.
///
/// One editor save typically emits several filesystem events; waiting for a
/// quiet period collapses them into a single reload.
pub(crate) const DEFAULT_DEBOUNCE_MS: u64 = 250;

/// Poll interval used when `poll` is given without one.
///
/// Two seconds is a compromise: fast enough that a deploy's configuration
/// change is picked up before anyone reaches for the logs, slow enough that a
/// dozen watched directories cost nothing measurable.
pub(crate) const DEFAULT_POLL_INTERVAL_MS: u64 = 2_000;

/// Validated arguments.
pub(crate) struct Args {
    /// Configuration files, merged left to right. May be empty when `search` is set.
    pub(crate) files: Vec<LitStr>,
    /// File name plus search directories, when configuration is discovered.
    pub(crate) search: Option<(LitStr, Vec<LitStr>)>,
    /// Environment variable prefix, e.g. `"APP_"`.
    pub(crate) env: Option<LitStr>,
    /// The section this struct maps to, e.g. `"db"`.
    pub(crate) key: LitStr,
    /// Whether to generate `start_watch()`.
    pub(crate) watch: bool,
    /// Debounce window in milliseconds. Meaningful only with `watch`.
    pub(crate) debounce: u64,
    /// Whether to generate the async methods.
    pub(crate) asynchronous: bool,
    /// Whether `FOO=` counts as set-to-empty.
    pub(crate) allow_empty_env: bool,
    /// Poll interval in milliseconds, when the native backend will not do.
    pub(crate) poll_interval: Option<u64>,
    /// Environment variable naming the active profile.
    pub(crate) profile_env: Option<LitStr>,
    /// Whether a load calls the type's own `validate`.
    pub(crate) validate: bool,
    /// Whether a reload logs the keys that changed.
    pub(crate) diff: bool,
    /// Separator that introduces nesting in a variable name.
    pub(crate) nest: Option<LitStr>,
    /// `.env` files, read as the environment layer.
    pub(crate) env_files: Vec<syn::LitStr>,
    /// Whether to generate `save`.
    pub(crate) save: bool,
    /// Whether to generate `schema`.
    pub(crate) schema: bool,
    /// Where to keep the last configuration that worked.
    pub(crate) cache: Option<LitStr>,
    /// How much of it to keep. `"full"` unless stated.
    pub(crate) cache_mode: Option<LitStr>,
}

/// Arguments as written, before validation.
///
/// Each field is optional so "not written" stays distinguishable from "written
/// with this value" — which is what makes duplicate detection and the
/// `debounce`-without-`watch` check possible. Spans travel with the values so
/// errors land on the offending token rather than the whole attribute.
#[derive(Default)]
struct Raw {
    files: Option<Vec<LitStr>>,
    name: Option<LitStr>,
    paths: Option<(Span, Vec<LitStr>)>,
    env: Option<LitStr>,
    key: Option<LitStr>,
    watch: Option<Span>,
    debounce: Option<(Span, u64)>,
    asynchronous: Option<Span>,
    allow_empty_env: Option<Span>,
    poll: Option<Span>,
    poll_interval: Option<(Span, u64)>,
    profile_env: Option<LitStr>,
    validate: Option<Span>,
    diff: Option<Span>,
    nest: Option<LitStr>,
    env_files: Option<Vec<syn::LitStr>>,
    save: Option<Span>,
    schema: Option<Span>,
    cache: Option<LitStr>,
    cache_mode: Option<LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut raw = Raw::default();

        while !input.is_empty() {
            // `parse_any`, so a keyword can be an argument name — `async` is the
            // honest name for the async surface now that it names no runtime.
            let name = input.call(Ident::parse_any)?;

            match name.to_string().as_str() {
                "files" => {
                    reject_duplicate(raw.files.is_some(), &name)?;
                    raw.files = Some(parse_string_array(input)?);
                }

                "name" => {
                    reject_duplicate(raw.name.is_some(), &name)?;
                    input.parse::<Token![=]>()?;
                    raw.name = Some(parse_non_empty(input, "`name`")?);
                }

                "paths" => {
                    reject_duplicate(raw.paths.is_some(), &name)?;
                    raw.paths = Some((name.span(), parse_string_array(input)?));
                }

                "env" => {
                    reject_duplicate(raw.env.is_some(), &name)?;
                    input.parse::<Token![=]>()?;
                    raw.env = Some(parse_non_empty(input, "`env` prefix")?);
                }

                "key" => {
                    reject_duplicate(raw.key.is_some(), &name)?;
                    input.parse::<Token![=]>()?;
                    raw.key = Some(parse_non_empty(input, "`key`")?);
                }

                // Bare flags: no `=`, no value.
                "watch" => {
                    reject_duplicate(raw.watch.is_some(), &name)?;
                    raw.watch = Some(name.span());
                }

                "async" => {
                    reject_duplicate(raw.asynchronous.is_some(), &name)?;
                    raw.asynchronous = Some(name.span());
                }

                "profile_env" => {
                    reject_duplicate(raw.profile_env.is_some(), &name)?;
                    input.parse::<Token![=]>()?;
                    raw.profile_env = Some(parse_non_empty(input, "`profile_env`")?);
                }

                "nest" => {
                    reject_duplicate(raw.nest.is_some(), &name)?;
                    input.parse::<Token![=]>()?;
                    raw.nest = Some(parse_non_empty(input, "`nest` separator")?);
                }

                "cache" => {
                    reject_duplicate(raw.cache.is_some(), &name)?;
                    input.parse::<Token![=]>()?;
                    raw.cache = Some(parse_non_empty(input, "`cache` path")?);
                }

                "cache_mode" => {
                    reject_duplicate(raw.cache_mode.is_some(), &name)?;
                    input.parse::<Token![=]>()?;

                    let mode = parse_non_empty(input, "`cache_mode`")?;

                    if !matches!(mode.value().as_str(), "full" | "redacted" | "fingerprint") {
                        return Err(Error::new(
                            mode.span(),
                            "unknown `cache_mode`; expected \"full\", \"redacted\" or \"fingerprint\"",
                        ));
                    }

                    raw.cache_mode = Some(mode);
                }

                "env_files" => {
                    reject_duplicate(raw.env_files.is_some(), &name)?;
                    raw.env_files = Some(parse_string_array(input)?);
                }

                "save" => {
                    reject_duplicate(raw.save.is_some(), &name)?;
                    raw.save = Some(name.span());
                }

                "schema" => {
                    reject_duplicate(raw.schema.is_some(), &name)?;
                    raw.schema = Some(name.span());
                }

                "validate" => {
                    reject_duplicate(raw.validate.is_some(), &name)?;
                    raw.validate = Some(name.span());
                }

                "diff" => {
                    reject_duplicate(raw.diff.is_some(), &name)?;
                    raw.diff = Some(name.span());
                }

                "poll" => {
                    reject_duplicate(raw.poll.is_some(), &name)?;
                    raw.poll = Some(name.span());
                }

                "poll_interval" => {
                    reject_duplicate(raw.poll_interval.is_some(), &name)?;
                    input.parse::<Token![=]>()?;

                    let literal: LitInt = input.parse()?;
                    let value: u64 = literal.base10_parse()?;

                    if value == 0 {
                        return Err(Error::new(
                            literal.span(),
                            "`poll_interval` must be greater than zero",
                        ));
                    }

                    raw.poll_interval = Some((literal.span(), value));
                }

                "allow_empty_env" => {
                    reject_duplicate(raw.allow_empty_env.is_some(), &name)?;
                    raw.allow_empty_env = Some(name.span());
                }

                "debounce" => {
                    reject_duplicate(raw.debounce.is_some(), &name)?;
                    input.parse::<Token![=]>()?;

                    let literal: LitInt = input.parse()?;
                    let value: u64 = literal.base10_parse()?;

                    if value == 0 {
                        return Err(Error::new(
                            literal.span(),
                            "`debounce` must be greater than zero",
                        ));
                    }

                    raw.debounce = Some((literal.span(), value));
                }

                other => {
                    return Err(Error::new(
                        name.span(),
                        format!(
                            "unknown argument `{other}`; \
                             expected one of: files, name, paths, key, env, watch, debounce, poll, \
                             poll_interval, profile_env, validate, diff, nest, env_files, \
                             save, schema, \
                             cache, \
                             cache_mode, async, allow_empty_env"
                        ),
                    ));
                }
            }

            // Trailing commas are allowed, so the separator is optional.
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        // `name` and `paths` are two halves of one thing: either alone would
        // silently search nowhere, or search for nothing.
        let search = match (raw.name, raw.paths) {
            (Some(name), Some((_, paths))) if !paths.is_empty() => Some((name, paths)),
            (Some(name), _) => {
                return Err(Error::new(
                    name.span(),
                    "`name` needs somewhere to look; add \
                     `paths = [\"/etc/myapp\", \"~/.config/myapp\", \".\"]`",
                ))
            }
            (None, Some((span, _))) => {
                return Err(Error::new(
                    span,
                    "`paths` needs something to look for; add `name = \"config\"`",
                ))
            }
            (None, None) => None,
        };

        // `files = []` is an explicit "no files, on purpose", which is a real
        // shape: a container whose configuration comes from a remote store and
        // the environment has no file to name. Omitting `files` entirely is
        // still an error, because that is a mistake rather than a decision.
        let stated_no_files = raw.files.as_ref().is_some_and(Vec::is_empty);
        let files = raw.files.unwrap_or_default();

        if files.is_empty() && search.is_none() && !stated_no_files {
            return Err(Error::new(
                Span::call_site(),
                "nothing to load; give `files = [\"config.toml\"]`, or \
                 `name = \"config\"` with `paths = [..]` to search for one — \
                 or `files = []` if the configuration comes from the environment \
                 and a remote store alone",
            ));
        }

        let key = raw.key.ok_or_else(|| {
            Error::new(
                Span::call_site(),
                "`key` is required; it names the config section this struct maps to, \
                 e.g. `key = \"db\"`",
            )
        })?;

        // Accepting `debounce` with the watcher disabled would silently do
        // nothing, which reads as a working hot-reload setup that never fires.
        if let (Some((span, _)), None) = (&raw.debounce, &raw.watch) {
            return Err(Error::new(
                *span,
                "`debounce` only applies to the file watcher; add `watch` or drop `debounce`",
            ));
        }

        // `diff` is deliberately not in this list. It describes what a *reload*
        // reports, and a reload now has two triggers: the file watcher, and a
        // document a remote watch pushed through `apply_remote`. Requiring
        // `watch` would rule out a program that only watches a store — which is
        // exactly the shape a container without a config file has.
        for (span, argument) in [
            (raw.poll.as_ref(), "poll"),
            (
                raw.poll_interval.as_ref().map(|(span, _)| span),
                "poll_interval",
            ),
        ] {
            if let (Some(span), None) = (span, &raw.watch) {
                return Err(Error::new(
                    *span,
                    format!("`{argument}` only applies to the file watcher; add `watch`"),
                ));
            }
        }

        // A mode with nowhere to write would silently do nothing.
        if let (Some(mode), None) = (&raw.cache_mode, &raw.cache) {
            return Err(Error::new(
                mode.span(),
                "`cache_mode` needs somewhere to write; add `cache = \"/var/lib/app/db.json\"`",
            ));
        }

        // With no environment layer there is nothing to separate.
        if let (Some(separator), None) = (&raw.nest, &raw.env) {
            return Err(Error::new(
                separator.span(),
                "`nest` only applies to the environment layer; \
                 add `env = \"..\"` or drop `nest`",
            ));
        }

        // Same reasoning: with no environment layer there is no empty variable
        // to decide about.
        if let (Some(span), None) = (&raw.allow_empty_env, &raw.env) {
            return Err(Error::new(
                *span,
                "`allow_empty_env` only applies to the environment layer; \
                 add `env = \"..\"` or drop `allow_empty_env`",
            ));
        }

        Ok(Self {
            files,
            search,
            env: raw.env,
            key,
            watch: raw.watch.is_some(),
            debounce: raw.debounce.map_or(DEFAULT_DEBOUNCE_MS, |(_, value)| value),
            asynchronous: raw.asynchronous.is_some(),
            allow_empty_env: raw.allow_empty_env.is_some(),
            profile_env: raw.profile_env,
            validate: raw.validate.is_some(),
            diff: raw.diff.is_some(),
            nest: raw.nest,
            env_files: raw.env_files.unwrap_or_default(),
            save: raw.save.is_some(),
            schema: raw.schema.is_some(),
            cache: raw.cache,
            cache_mode: raw.cache_mode,
            poll_interval: match (raw.poll, raw.poll_interval) {
                (_, Some((_, interval))) => Some(interval),
                (Some(_), None) => Some(DEFAULT_POLL_INTERVAL_MS),
                (None, None) => None,
            },
        })
    }
}

fn reject_duplicate(already_seen: bool, name: &Ident) -> Result<()> {
    if already_seen {
        return Err(Error::new(
            name.span(),
            format!("duplicate `{name}` argument"),
        ));
    }

    Ok(())
}

fn parse_non_empty(input: ParseStream<'_>, what: &str) -> Result<LitStr> {
    let literal: LitStr = input.parse()?;

    if literal.value().is_empty() {
        return Err(Error::new(
            literal.span(),
            format!("{what} must not be empty"),
        ));
    }

    Ok(literal)
}

/// Parses `= ["a", "b"]`.
fn parse_string_array(input: ParseStream<'_>) -> Result<Vec<LitStr>> {
    input.parse::<Token![=]>()?;

    let content;
    syn::bracketed!(content in input);

    let mut values = Vec::new();

    while !content.is_empty() {
        values.push(content.parse::<LitStr>()?);

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    //! The trybuild suites prove errors *render* correctly; these prove the
    //! parsing *decisions*, cheaply enough to run on every keystroke.

    use super::*;

    // `Args` has no `Debug` — nothing but these tests would use one — so the
    // helpers unwrap without asking for it.
    fn parse_ok(arguments: &str) -> Args {
        match syn::parse_str::<Args>(arguments) {
            Ok(args) => args,
            Err(error) => panic!("`{arguments}` should parse: {error}"),
        }
    }

    fn parse_err(arguments: &str) -> Error {
        syn::parse_str::<Args>(arguments)
            .err()
            .unwrap_or_else(|| panic!("`{arguments}` should be rejected"))
    }

    #[test]
    fn a_duplicate_argument_names_itself() {
        let error = parse_err(r#"files = ["a.json"], key = "db", key = "web""#);

        assert!(error.to_string().contains("duplicate `key`"), "{error}");
    }

    #[test]
    fn name_and_paths_must_travel_together() {
        let alone = parse_err(r#"name = "config", key = "db""#);
        assert!(alone.to_string().contains("somewhere to look"), "{alone}");

        let other = parse_err(r#"paths = ["/etc/app"], key = "db""#);
        assert!(
            other.to_string().contains("something to look for"),
            "{other}"
        );
    }

    #[test]
    fn debounce_without_watch_is_refused_rather_than_ignored() {
        let error = parse_err(r#"files = ["a.json"], key = "db", debounce = 100"#);

        assert!(error.to_string().contains("add `watch`"), "{error}");
    }

    #[test]
    fn poll_without_watch_is_refused_the_same_way() {
        let error = parse_err(r#"files = ["a.json"], key = "db", poll"#);

        assert!(error.to_string().contains("add `watch`"), "{error}");
    }

    #[test]
    fn cache_mode_accepts_only_the_three_modes() {
        let error =
            parse_err(r#"files = ["a.json"], key = "db", cache = "c.json", cache_mode = "shiny""#);

        assert!(
            error.to_string().contains("unknown `cache_mode`"),
            "{error}"
        );

        for mode in ["full", "redacted", "fingerprint"] {
            let arguments = format!(
                r#"files = ["a.json"], key = "db", cache = "c.json", cache_mode = "{mode}""#
            );

            assert!(
                syn::parse_str::<Args>(&arguments).is_ok(),
                "`{mode}` is a real mode"
            );
        }
    }

    #[test]
    fn an_empty_files_list_is_a_decision_and_omitting_it_is_a_mistake() {
        parse_ok(r#"files = [], key = "db""#);

        let error = parse_err(r#"key = "db""#);
        assert!(error.to_string().contains("nothing to load"), "{error}");
    }

    #[test]
    fn env_files_parse_into_the_list_they_were_written_as() {
        let args =
            parse_ok(r#"files = ["a.json"], key = "db", env_files = [".env", ".env.local"]"#);

        let paths: Vec<String> = args.env_files.iter().map(LitStr::value).collect();
        assert_eq!(paths, [".env", ".env.local"]);
    }

    #[test]
    fn poll_alone_gets_the_default_interval() {
        let args = parse_ok(r#"files = ["a.json"], key = "db", watch, poll"#);

        assert_eq!(args.poll_interval, Some(DEFAULT_POLL_INTERVAL_MS));

        let explicit = parse_ok(r#"files = ["a.json"], key = "db", watch, poll_interval = 500"#);
        assert_eq!(explicit.poll_interval, Some(500));
    }

    #[test]
    fn nest_and_allow_empty_env_need_the_env_layer() {
        let nest = parse_err(r#"files = ["a.json"], key = "db", nest = "__""#);
        assert!(nest.to_string().contains("environment layer"), "{nest}");

        let empty = parse_err(r#"files = ["a.json"], key = "db", allow_empty_env"#);
        assert!(empty.to_string().contains("environment layer"), "{empty}");
    }

    #[test]
    fn an_unknown_argument_lists_what_exists() {
        let error = parse_err(r#"files = ["a.json"], key = "db", wach"#);

        let rendered = error.to_string();
        assert!(rendered.contains("unknown argument `wach`"), "{rendered}");
        assert!(rendered.contains("watch"), "{rendered}");
    }
}
