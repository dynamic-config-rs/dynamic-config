//! Finding configuration files instead of being told where they are.
//!
//! A hardcoded `files = ["config.toml"]` resolves against the working
//! directory, which is fine for a repository checkout and wrong for everything
//! else: a package puts its configuration in `/etc`, a user overrides it in
//! `~/.config`, and a developer overrides *that* in the current directory.
//!
//! ```text
//! name  = "config"
//! paths = ["/etc/myapp", "~/.config/myapp", "."]
//!         →  /etc/myapp/config.toml
//!            ~/.config/myapp/config.json
//!            ./config.yaml
//! ```
//!
//! Every directory that has a match contributes one, merged in the order the
//! paths are listed — so the layering is the search order, and the last
//! directory wins. This is where it differs from Go's Viper, which stops at the
//! first hit: the whole reason to list `/etc` *and* `~` is to layer them.
//!
//! Within one directory the extensions are tried in a fixed order and the first
//! hit is taken, so a stray `config.json` next to a `config.toml` is resolved
//! the same way every run rather than by directory-listing order.

#[cfg(feature = "watch")]
use std::path::Path;
use std::path::PathBuf;

use crate::source::Format;

/// Extensions tried in each directory, in order. Formats whose feature is off
/// are skipped, so a build without `yaml` never picks up a `config.yaml` it
/// could not parse.
const EXTENSIONS: &[(&str, Format)] = &[
    #[cfg(feature = "toml")]
    ("toml", Format::Toml),
    #[cfg(feature = "json")]
    ("json", Format::Json),
    #[cfg(feature = "yaml")]
    ("yaml", Format::Yaml),
    #[cfg(feature = "yaml")]
    ("yml", Format::Yaml),
];

/// Where to look for configuration, and under what name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Search<'a> {
    /// The file name without an extension, e.g. `"config"`.
    pub name: &'a str,
    /// Directories to search, in increasing order of precedence.
    pub paths: &'a [&'a str],
}

impl<'a> Search<'a> {
    /// Looks for `{name}.{ext}` in each of `paths`.
    #[must_use]
    pub const fn new(name: &'a str, paths: &'a [&'a str]) -> Self {
        Self { name, paths }
    }

    /// The files that exist right now, in merge order.
    ///
    /// Resolution happens per load rather than once, so a file that appears
    /// later — a mounted secret, a freshly written override — is picked up by
    /// the next reload instead of requiring a restart.
    pub(crate) fn resolve(&self) -> Vec<(PathBuf, Format)> {
        let mut found = Vec::new();

        for directory in self.paths {
            let Some(directory) = expand_home(directory) else {
                continue;
            };

            for (extension, format) in EXTENSIONS {
                let candidate = directory.join(format!("{}.{extension}", self.name));

                if candidate.is_file() {
                    found.push((candidate, *format));
                    break;
                }
            }
        }

        found
    }
}

/// Expands a leading `~` using `HOME`, or `USERPROFILE` on Windows.
///
/// A `~` that cannot be expanded drops the directory rather than searching a
/// literal `./~`, which would be a confusing place to find configuration.
fn expand_home(path: &str) -> Option<PathBuf> {
    let Some(rest) = path.strip_prefix('~') else {
        return Some(PathBuf::from(path));
    };

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;

    let rest = rest.trim_start_matches(['/', '\\']);

    Some(if rest.is_empty() {
        home
    } else {
        home.join(rest)
    })
}

/// Whether `path` looks like a file this build could parse.
///
/// Used by the watcher, which has to decide what to watch before any file
/// exists.
#[cfg(feature = "watch")]
pub(crate) fn is_candidate(path: &Path, name: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };

    if stem != name {
        return false;
    }

    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(Format::from_extension)
        .is_some()
}

/// Directories a watcher should listen to for this search.
///
/// Every listed directory, existing or not — a config file that appears later
/// is exactly the event worth catching, and watching a directory that is absent
/// simply fails and is reported.
#[cfg(feature = "watch")]
pub(crate) fn search_directories(search: &Search<'_>) -> Vec<PathBuf> {
    search
        .paths
        .iter()
        .filter_map(|path| expand_home(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("dynamic-config-discovery-{name}"));

        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("the scratch directory should be creatable");

        directory
    }

    #[test]
    fn a_directory_with_no_match_contributes_nothing() {
        let directory = scratch("empty");
        let path = directory.to_string_lossy().into_owned();

        assert!(Search::new("config", &[&path]).resolve().is_empty());
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_matching_file_is_found_with_its_format() {
        let directory = scratch("one");
        fs::write(directory.join("config.json"), "{}").unwrap();

        let path = directory.to_string_lossy().into_owned();
        let found = Search::new("config", &[&path]).resolve();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, Format::Json);
    }

    #[cfg(all(feature = "json", feature = "toml"))]
    #[test]
    fn one_directory_contributes_at_most_one_file() {
        let directory = scratch("ambiguous");
        fs::write(directory.join("config.json"), "{}").unwrap();
        fs::write(directory.join("config.toml"), "").unwrap();

        let path = directory.to_string_lossy().into_owned();
        let found = Search::new("config", &[&path]).resolve();

        assert_eq!(
            found.len(),
            1,
            "extension order decides, not the filesystem"
        );
        assert_eq!(
            found[0].1,
            Format::Toml,
            "`toml` is tried before `json`, so the result is stable"
        );
    }

    #[cfg(feature = "json")]
    #[test]
    fn directories_are_searched_in_the_order_given() {
        let first = scratch("first");
        let second = scratch("second");
        fs::write(first.join("config.json"), "{}").unwrap();
        fs::write(second.join("config.json"), "{}").unwrap();

        let first_path = first.to_string_lossy().into_owned();
        let second_path = second.to_string_lossy().into_owned();
        let found = Search::new("config", &[&first_path, &second_path]).resolve();

        assert_eq!(found.len(), 2, "each directory contributes its own layer");
        assert_eq!(found[0].0.parent().unwrap(), first);
        assert_eq!(found[1].0.parent().unwrap(), second);
    }

    #[test]
    fn a_leading_tilde_expands_to_the_home_directory() {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);

        let Some(home) = home else {
            return;
        };

        assert_eq!(expand_home("~"), Some(home.clone()));
        assert_eq!(expand_home("~/.config/app"), Some(home.join(".config/app")));
    }

    #[test]
    fn a_path_without_a_tilde_is_left_alone() {
        assert_eq!(expand_home("/etc/app"), Some(PathBuf::from("/etc/app")));
        assert_eq!(expand_home("."), Some(PathBuf::from(".")));
    }

    #[cfg(feature = "watch")]
    #[test]
    fn the_watcher_recognises_a_candidate_by_stem_and_extension() {
        assert!(is_candidate(Path::new("/etc/app/config.json"), "config"));
        assert!(is_candidate(Path::new("config.yml"), "config"));
        assert!(!is_candidate(Path::new("/etc/app/other.json"), "config"));
        assert!(!is_candidate(Path::new("/etc/app/config.ini"), "config"));
        assert!(!is_candidate(Path::new("/etc/app/config"), "config"));
    }
}
