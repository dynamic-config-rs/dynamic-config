//! Recovery: loading with the cache standing in for the files that broke.

use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::snapshot::Snapshot;
use crate::source::LoadSpec;

use super::aliases_pass::apply_aliases;

/// Loads with `cached` standing in for the files.
///
/// The files are what broke, so recovery reads none of them — a malformed file
/// fails to parse whatever sits underneath it. The environment and the runtime
/// layers are applied over the cache exactly as they would be over the files,
/// which is what lets a redacted cache work: the values it dropped come back
/// from wherever they were live.
pub(crate) fn recover<T: DeserializeOwned>(
    spec: &LoadSpec<'_>,
    cached: &Snapshot,
) -> Result<(T, Snapshot), Error> {
    // The cache stands in for the files, at the bottom of the order: what it
    // holds is what those files last said, and everything the environment and
    // the runtime layers have to say still goes over the top of it.
    let mut collected = crate::resolve::Collected::default();
    collected.layer(
        "cached",
        crate::Origin::Runtime("cached configuration"),
        cached.values().clone(),
    );

    // The same order as a load, deliberately: `.env` files first, the real
    // environment after, so a variable somebody exported to steer the
    // recovery beats a file in the repository — recovery is exactly the
    // moment a human is overriding things by hand.
    super::environment::collect_env_files(&mut collected, spec)?;
    super::collect_environment(&mut collected, spec)?;
    super::collect_bindings(&mut collected, spec)?;
    super::collect_runtime(&mut collected, spec);

    let (mut tree, mut provenance) =
        crate::resolve::compose(spec.engine(), collected.take_layers())?;

    apply_aliases(&mut tree, &mut provenance, &collected, spec)?;

    let mut snapshot = Snapshot::new(tree);
    snapshot.attach_provenance(provenance);

    // The resolved tree rides along so the caller can seed the diff baseline:
    // without it, the first successful reload after a recovery had nothing to
    // compare against and reported no changes — untrue, and at the worst time.
    let config: T = snapshot.extract()?;

    Ok((config, snapshot))
}
