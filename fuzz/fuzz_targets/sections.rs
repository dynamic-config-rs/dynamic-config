//! The section and profile machinery, structure-aware: a file name and a
//! profile, not bytes.
//!
//! Two surfaces, both reached from outside the process. A *section* is a
//! top-level key in a configuration document, turned into the figment profile
//! the loader files it under. A *profile* is a word out of an environment
//! variable, interpolated into a file name — `config.toml` plus `production`
//! becomes `config.production.toml`, which the loader then reads.
//!
//! The second one is where the 0.4 traversal bug lived: `APP_ENV=../../etc`
//! walked the loader out of the configuration directory and merged whatever
//! it found there, *above* the base file. The fix is a pair — a guard
//! (`profile_is_safe`) and the interpolation it guards (`profile_variant`) —
//! and a pair is only correct together. So the property here is the pair:
//!
//! * **anything `profile_is_safe` accepts leaves `profile_variant` naming a
//!   path in the same directory it started in.** A profile that escapes its
//!   directory is a traversal, whether it arrives as `..`, as a separator, or
//!   as something nobody has thought of yet.
//! * The profile lands in the *file name*, not somewhere else in the path —
//!   an escape that put it in a directory component would satisfy a laxer
//!   reading of the first property.
//! * The variant has the same number of components as its base, which is the
//!   same claim from the other side: no component was added or dropped.
//!
//! And for sections:
//!
//! * a section profile is never one of figment's own reserved profiles. This
//!   is the other 0.4 bug: an unprefixed mapping handed `global` and
//!   `default` — which figment treats as merge-into-everything and
//!   fall-back-for-everything — to any document with an innocently named
//!   table.
//! * Two section keys land on one profile only if they differ by ASCII case
//!   alone. Anything more than that is two tables silently merging into one.
//!   Case itself is excluded because figment compares profiles case-
//!   insensitively; `db` and `DB` naming one section is a figment property,
//!   not a defect here.
//!
//! Names are generated as *shapes* — directories, a stem, a stack of
//! extensions — because the branch worth reaching is the encrypted one, where
//! `secrets.json.age` has to become `secrets.production.json.age` rather than
//! `secrets.json.production.age`. Discovering `.age` from bytes would cost the
//! generator its whole budget. `Name::Raw` keeps everything else reachable.

#![no_main]

use std::path::Path;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// A file name the generator can build, plus an escape hatch for text that is
/// not shaped like one.
#[derive(Arbitrary, Debug)]
enum Name {
    Structured {
        directories: Vec<String>,
        stem: String,
        /// Rendered dot-separated after the stem, so `["json", "age"]` gives
        /// the encrypted shape the recursive branch handles.
        extensions: Vec<String>,
    },
    Raw(String),
}

impl Name {
    fn render(&self) -> String {
        match self {
            Name::Structured {
                directories,
                stem,
                extensions,
            } => {
                let mut path = String::new();

                for directory in directories {
                    path.push_str(directory);
                    path.push('/');
                }

                path.push_str(stem);

                for extension in extensions {
                    path.push('.');
                    path.push_str(extension);
                }

                path
            }
            Name::Raw(text) => text.clone(),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    path: Name,
    profile: String,
    /// A section key, and a second one to check the two against each other.
    key: String,
    other: String,
}

fuzz_target!(|input: Input| {
    let path = input.path.render();
    let profile = &input.profile;

    if dynamic_config::__fuzz::profile_is_safe(profile) {
        if let Some(variant) = dynamic_config::__fuzz::profile_variant(&path, profile) {
            let base = Path::new(&path);
            let built = Path::new(&variant);

            // The whole of the traversal fix, stated as one equality: the
            // variant is a sibling of the file it varies.
            assert_eq!(
                built.parent(),
                base.parent(),
                "profile {profile:?} moved {path:?} to {variant:?}"
            );

            assert_eq!(
                built.components().count(),
                base.components().count(),
                "profile {profile:?} changed the depth of {path:?}: {variant:?}"
            );

            // Same claim once more, from the side an attacker would attack:
            // the profile has to have landed in the file name. A variant
            // that is a sibling but spelled the profile into a directory
            // component would be a traversal waiting for the next caller.
            let file_name = built
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a variant is built by naming a file");

            assert!(
                file_name.contains(profile.as_str()),
                "profile {profile:?} is not in the name of {variant:?}"
            );
        }
    }

    let section = dynamic_config::__fuzz::section_profile(&input.key);

    // figment merges `global` into every profile and falls back to `default`,
    // so a section landing on either is a document silently reconfiguring
    // every other section. The prefix is what keeps them apart; comparison is
    // case-insensitive because figment's own is.
    for reserved in ["global", "default"] {
        assert!(
            !section.eq_ignore_ascii_case(reserved),
            "section {:?} became figment's reserved {reserved:?} profile",
            input.key
        );
    }

    assert!(
        section.contains(input.key.as_str()),
        "section {:?} is not recoverable from the profile {section:?}",
        input.key
    );

    // Distinct sections stay distinct. Two keys sharing a profile would merge
    // their tables into one, which no document asked for.
    let other = dynamic_config::__fuzz::section_profile(&input.other);

    if section.eq_ignore_ascii_case(&other) {
        assert!(
            input.key.eq_ignore_ascii_case(&input.other),
            "sections {:?} and {:?} share the profile {section:?}",
            input.key,
            input.other
        );
    }
});
