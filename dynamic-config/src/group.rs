//! Reloading several configuration types as one step.
//!
//! Two structs over one file reload independently: for a moment after an edit,
//! `ServerConfig` is new while `DbConfig` is still old. Usually nobody notices.
//! Sometimes it matters — a TLS certificate path and the port it is served on,
//! a queue name and the credentials for it — and then a half-applied
//! configuration is worse than a late one.
//!
//! A group fixes that by splitting a reload in two. **Prepare** does everything
//! that can fail: read, merge, deserialize, validate. **Commit** does the part
//! that cannot: swap an `Arc`. Every member prepares before any member commits,
//! so a failure anywhere leaves every member on its previous snapshot.
//!
//! ```text
//! prepare A ─┐
//! prepare B ─┼─ all succeeded? ─ yes ─→ commit A, commit B
//! prepare C ─┘                   no  ─→ nothing moves
//! ```
//!
//! The commits are not one atomic operation — nothing in `std` makes three
//! `Arc` swaps simultaneous — but they happen with no fallible work between
//! them, which is the part that actually goes wrong.

use crate::error::Error;

/// The second half of a reload: the part that cannot fail.
pub type Commit = Box<dyn FnOnce() + Send>;

/// A configuration type that a [`ReloadGroup`] can drive.
///
/// Implemented by `#[dynamic_config]`. Implementing it by hand is possible but
/// rarely what you want — the contract is that `prepare` does *all* the
/// fallible work, and nothing checks that for you.
pub trait Reloadable: 'static {
    /// Loads and validates, returning the swap to perform.
    ///
    /// # Errors
    ///
    /// Whatever a load of this type would report.
    fn prepare() -> Result<Commit, Error>;

    /// The type's name, for diagnostics.
    fn name() -> &'static str;
}

/// Several configuration types that reload together or not at all.
///
/// ```no_run
/// # use dynamic_config::ReloadGroup;
/// # struct ServerConfig; struct DbConfig;
/// # impl dynamic_config::Reloadable for ServerConfig {
/// #     fn prepare() -> Result<dynamic_config::Commit, dynamic_config::Error> { unimplemented!() }
/// #     fn name() -> &'static str { "ServerConfig" }
/// # }
/// # impl dynamic_config::Reloadable for DbConfig {
/// #     fn prepare() -> Result<dynamic_config::Commit, dynamic_config::Error> { unimplemented!() }
/// #     fn name() -> &'static str { "DbConfig" }
/// # }
/// let group = ReloadGroup::new()
///     .with::<ServerConfig>()
///     .with::<DbConfig>();
///
/// group.reload()?;
/// # Ok::<(), dynamic_config::Error>(())
/// ```
#[derive(Default)]
pub struct ReloadGroup {
    members: Vec<Member>,
    /// Serializes [`reload`](Self::reload); see there.
    reloading: std::sync::Mutex<()>,
}

struct Member {
    name: &'static str,
    prepare: fn() -> Result<Commit, Error>,
}

impl ReloadGroup {
    /// An empty group.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            members: Vec::new(),
            reloading: std::sync::Mutex::new(()),
        }
    }

    /// Adds a configuration type.
    ///
    /// Order matters only for which failure is reported first; the outcome is
    /// all-or-nothing either way.
    #[must_use]
    pub fn with<T: Reloadable>(mut self) -> Self {
        self.members.push(Member {
            name: T::name(),
            prepare: T::prepare,
        });

        self
    }

    /// The types in this group, in the order they were added.
    pub fn members(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.members.iter().map(|member| member.name)
    }

    /// Whether the group would do anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Loads every member, then installs every member.
    ///
    /// # Errors
    ///
    /// The first failure, with the offending type's name prepended. Nothing has
    /// been installed when this returns an error — not even the members that
    /// loaded cleanly.
    pub fn reload(&self) -> Result<(), Error> {
        // Serialized: two concurrent reloads — the admin endpoint and the
        // file watcher, say — could otherwise interleave their commit loops
        // and leave member A on one thread's prepare and member B on the
        // other's, which is the exact mixed state this type exists to
        // prevent.
        let _guard = self
            .reloading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut commits = Vec::with_capacity(self.members.len());

        for member in &self.members {
            // Any failure here drops the commits collected so far, and dropping
            // a commit is what *not* applying it means.
            let commit = (member.prepare)().map_err(|error| error.prepend_key(member.name))?;

            commits.push(commit);
        }

        for commit in commits {
            // Caught per commit: a commit ends in `store`, which runs reload
            // hooks, and a panicking hook must not stop the *other members'*
            // snapshots from installing — a half-committed group is the
            // failure this type promises against. (The hook itself is also
            // caught in `dispatch`; this is the second belt for anything
            // that unwinds out of a commit some other way.)
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(commit));

            if outcome.is_err() {
                crate::log::warning!(
                    "a commit's reload hook panicked; the remaining members \
                     were still committed"
                );
            }
        }

        Ok(())
    }
}

impl std::fmt::Debug for ReloadGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.members()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A counter per test rather than one shared by all of them: these run in
    /// parallel, and a `static` they all reset is a race that passes until it
    /// does not.
    macro_rules! members {
        ($counter:ident, $good:ident, $bad:ident) => {
            static $counter: AtomicUsize = AtomicUsize::new(0);

            struct $good;
            // Not every test needs the failing half; the macro declares both so
            // each test's types stay independent.
            #[allow(dead_code)]
            struct $bad;

            impl Reloadable for $good {
                fn prepare() -> Result<Commit, Error> {
                    Ok(Box::new(|| {
                        $counter.fetch_add(1, Ordering::SeqCst);
                    }))
                }

                fn name() -> &'static str {
                    stringify!($good)
                }
            }

            impl Reloadable for $bad {
                fn prepare() -> Result<Commit, Error> {
                    Err(Error::new(crate::ErrorKind::Missing, "nothing supplies it"))
                }

                fn name() -> &'static str {
                    stringify!($bad)
                }
            }
        };
    }

    members!(ALL_COMMITTED, AllGood, AllBad);
    members!(NONE_COMMITTED, NoneGood, NoneBad);
    members!(ORDER_COMMITTED, OrderGood, OrderBad);

    #[test]
    fn every_member_commits_when_every_member_prepares() {
        ReloadGroup::new()
            .with::<AllGood>()
            .with::<AllGood>()
            .reload()
            .expect("both prepare cleanly");

        assert_eq!(ALL_COMMITTED.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn one_failure_stops_every_commit_including_the_ones_that_would_have_worked() {
        let error = ReloadGroup::new()
            .with::<NoneGood>()
            .with::<NoneBad>()
            .with::<NoneGood>()
            .reload()
            .expect_err("the middle member fails");

        assert_eq!(
            NONE_COMMITTED.load(Ordering::SeqCst),
            0,
            "the member that prepared before the failure must not have committed"
        );
        assert!(error.path().starts_with("NoneBad"), "{error}");
    }

    #[test]
    fn an_empty_group_is_a_no_op() {
        let group = ReloadGroup::new();

        assert!(group.is_empty());
        assert!(group.reload().is_ok());
    }

    #[test]
    fn a_group_reports_its_members_in_order() {
        let group = ReloadGroup::new().with::<OrderGood>().with::<OrderBad>();

        assert_eq!(
            group.members().collect::<Vec<_>>(),
            ["OrderGood", "OrderBad"]
        );
    }
}
