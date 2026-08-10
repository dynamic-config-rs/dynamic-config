//! Per-monomorphization storage for generic configuration types.
//!
//! A non-generic config type keeps its snapshot in a `static`, which costs one
//! atomic load to read. A generic one cannot: Rust has no generic statics, and
//! `Config<Postgres>` and `Config<Mysql>` need separate snapshots.
//!
//! The way out is a registry keyed by [`TypeId`], which *is* per
//! monomorphization. A `static Registry` inside a generic function is shared by
//! every instantiation — items in a function body are not monomorphized — so
//! one registry serves them all and the key tells them apart.
//!
//! ```text
//! non-generic:  static CELL: ConfigCell<T>        →  one atomic load
//! generic:      REGISTRY.entry::<Config<D>, _>()  →  read lock + hash + downcast
//! ```
//!
//! That difference is why the macro emits the `static` whenever it can, and the
//! registry only for the types that have no alternative. `benches/read_path.rs`
//! measures both.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;

use arc_swap::ArcSwap;

/// The table itself. Behind an [`ArcSwap`], so a read takes no lock at all.
type Slots = HashMap<TypeId, &'static (dyn Any + Send + Sync), BuildHasherDefault<TypeIdHasher>>;

/// Passes a [`TypeId`] straight through instead of hashing it.
///
/// `TypeId` is already a high-quality 128-bit value; running SipHash over it
/// was measurably the largest part of a lookup, and buys nothing a compiler-
/// generated identifier does not already have.
#[derive(Default)]
struct TypeIdHasher {
    hash: u64,
}

impl Hasher for TypeIdHasher {
    fn write(&mut self, bytes: &[u8]) {
        // `TypeId`'s `Hash` writes its bytes in one go; taking the low eight is
        // enough to spread the handful of keys a program actually has.
        for chunk in bytes.chunks(8) {
            let mut buffer = [0u8; 8];
            buffer[..chunk.len()].copy_from_slice(chunk);

            self.hash ^= u64::from_ne_bytes(buffer);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.hash ^= value;
    }

    fn write_u128(&mut self, value: u128) {
        self.hash ^= (value as u64) ^ ((value >> 64) as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

/// One slot per type, allocated on first use and never freed.
///
/// Each generated accessor has its own `Registry`, so the snapshot, the two
/// runtime layers and the diff baseline do not collide on a shared key.
///
/// # Example
///
/// ```
/// use dynamic_config::{ConfigCell, Registry};
///
/// fn cell<T: Send + Sync + 'static>() -> &'static ConfigCell<T> {
///     // Shared by every instantiation: a `static` in a function body is not
///     // monomorphized.
///     static REGISTRY: Registry = Registry::new();
///
///     REGISTRY.entry::<T, ConfigCell<T>>()
/// }
///
/// cell::<u16>().store(8080);
/// assert_eq!(*cell::<u16>().load().unwrap(), 8080);
///
/// // A different type is a different slot.
/// assert!(cell::<String>().load().is_none());
/// ```
#[derive(Default)]
pub struct Registry {
    entries: OnceLock<ArcSwap<Slots>>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: OnceLock::new(),
        }
    }

    /// The slot for `K`, created on first use.
    ///
    /// The value is leaked deliberately. A configuration snapshot lives as long
    /// as the process, and the alternative — handing out an `Arc` and reference
    /// counting on every read — would cost more than the lookup it replaced.
    /// The leak is bounded by the number of monomorphizations, which is fixed
    /// at compile time.
    ///
    /// # Panics
    ///
    /// If one registry is used with two different `V` types for the same `K`
    /// — the generated code never does this; hand-written callers must keep
    /// one value type per registry.
    pub fn entry<K, V>(&self) -> &'static V
    where
        K: 'static,
        V: Default + Send + Sync + 'static,
    {
        let entries = self
            .entries
            .get_or_init(|| ArcSwap::from_pointee(Slots::default()));
        let key = TypeId::of::<K>();

        // The common case by far, and the one the benchmark measures: no lock,
        // no hashing beyond a load, no allocation.
        if let Some(found) = entries.load().get(&key) {
            return downcast(*found);
        }

        // Missing: rebuild the table with the new slot in it. This happens once
        // per monomorphization, so copying a table with a handful of entries is
        // cheaper than making every read pay for a lock. A lost race leaks
        // this one allocation — bounded by the same monomorphization count as
        // the deliberate leak above, so it is a footnote, not a leak *rate*.
        let leaked: &'static V = Box::leak(Box::new(V::default()));
        let mut installed: Option<&'static (dyn Any + Send + Sync)> = None;

        entries.rcu(|current| {
            let mut next = Slots::clone(current);
            // `or_insert` rather than `insert`: another thread may have won the
            // race, and two slots for one type would mean two snapshots.
            installed = Some(*next.entry(key).or_insert(leaked));

            next
        });

        downcast(installed.expect("`rcu` runs its closure at least once"))
    }
}

/// Recovers the concrete type a slot was created with.
///
/// The key is `TypeId::of::<K>()` and each registry serves exactly one `V`, so
/// a mismatch would mean two different `V`s reached the same registry — a bug
/// in the generated code rather than anything a caller can cause.
fn downcast<V: 'static>(slot: &'static (dyn Any + Send + Sync)) -> &'static V {
    slot.downcast_ref()
        .expect("a registry slot holds exactly one type")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    #[derive(Default)]
    struct Slot(AtomicUsize);

    fn registry() -> &'static Registry {
        static REGISTRY: Registry = Registry::new();

        &REGISTRY
    }

    #[test]
    fn the_same_key_always_returns_the_same_slot() {
        let first = registry().entry::<u8, Slot>();
        first.0.store(7, Ordering::SeqCst);

        let second = registry().entry::<u8, Slot>();

        assert!(std::ptr::eq(first, second));
        assert_eq!(second.0.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn different_keys_get_different_slots() {
        let one = registry().entry::<u16, Slot>();
        let two = registry().entry::<u32, Slot>();

        assert!(!std::ptr::eq(one, two));
    }

    #[test]
    fn concurrent_first_use_hands_out_one_slot() {
        struct Contended;

        let handles: Vec<_> = (0..8)
            .map(|_| {
                thread::spawn(|| {
                    // A raw pointer is not `Send`, so the address travels back
                    // as an integer instead.
                    registry().entry::<Contended, Slot>() as *const Slot as usize
                })
            })
            .collect();

        let slots: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        assert!(
            slots.windows(2).all(|pair| pair[0] == pair[1]),
            "every thread must see the same slot"
        );
    }
}
