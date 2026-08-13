//! `touches_secret`, structure-aware: a path and a secret list, not bytes.
//!
//! This is the target that matters most. `touches_secret` is what stands
//! between a resolved password and a diagnostic, a cache file or an
//! `explain` report — the code whose failure mode is a secret on disk. It
//! takes exactly the input an attacker supplies: key names out of a
//! configuration document.
//!
//! Bytes into it would be a waste, so the generator produces *shapes*: dotted
//! paths and dotted secret names, including the cases that are interesting on
//! purpose — a secret that is a prefix of another key, a name that contains
//! a dot, an empty segment.
//!
//! The properties are the ones the redaction argument rests on. Each is one
//! the three-disjunct implementation could plausibly get wrong:
//!
//! * an empty secret list covers nothing;
//! * a path always covers itself;
//! * coverage is symmetric — `explain("credentials")` must redact when
//!   `credentials.password` is secret, and vice versa;
//! * everything *below* a secret is the secret's;
//! * adding a secret never uncovers a path.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// A dotted name, assembled from segments so the generator spends its budget
/// on tree shape rather than on discovering that `.` is the separator.
#[derive(Arbitrary, Debug)]
struct Name {
    segments: Vec<String>,
}

impl Name {
    fn render(&self) -> String {
        self.segments.join(".")
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    path: Name,
    secrets: Vec<Name>,
    /// Appended to a secret to build a path that must be covered.
    below: Name,
}

fuzz_target!(|input: Input| {
    let path = input.path.render();
    let secrets: Vec<String> = input.secrets.iter().map(Name::render).collect();

    let covered = dynamic_config::touches_secret(&path, &secrets);

    let none: [String; 0] = [];
    assert!(
        !dynamic_config::touches_secret(&path, &none),
        "an empty secret list covers nothing, but covered {path:?}"
    );

    assert!(
        dynamic_config::touches_secret(&path, &[path.clone()]),
        "a path must cover itself: {path:?}"
    );

    for secret in &secrets {
        // Symmetry. The ancestor case and the descendant case are separate
        // branches in the implementation, and a change that fixes one and
        // forgets the other is caught here rather than in an incident.
        assert_eq!(
            dynamic_config::touches_secret(&path, &[secret.clone()]),
            dynamic_config::touches_secret(secret, &[path.clone()]),
            "coverage must be symmetric: {path:?} against {secret:?}"
        );

        // Everything below a secret is the secret's. Skipped when the tail is
        // empty: `secret.` is not a path any source produces, and the
        // separator would be dangling.
        let tail = input.below.render();

        if !tail.is_empty() {
            let deeper = format!("{secret}.{tail}");

            assert!(
                dynamic_config::touches_secret(&deeper, &[secret.clone()]),
                "a path below a secret is the secret's: {deeper:?} under {secret:?}"
            );
        }
    }

    // Monotone: a longer secret list covers at least as much. A change that
    // made coverage depend on list order would show up here.
    if covered {
        let mut widened = secrets.clone();
        widened.push(String::from("an.unrelated.name"));

        assert!(
            dynamic_config::touches_secret(&path, &widened),
            "adding a secret must not uncover {path:?}"
        );
    }
});
