//! Which events are about our files.
//!
//! The whole directory is watched, so most events are about something
//! else; everything here is the filter, including the Kubernetes
//! ConfigMap `..data` convention that makes a remount visible at all.

use std::path::{Path, PathBuf};

use notify::{Event, EventKind};

use crate::discovery;

use super::Watched;

/// Which of an event's paths is one of ours, if any.
///
/// The whole directory is watched, so most events are about something else.
/// Paths are compared in both directions because event paths are absolute while
/// configured paths are usually relative to the working directory; a rare false
/// positive costs one redundant reload, which is harmless.
///
/// The path is returned rather than a bare `yes`, because the reload it
/// causes wants to say *which file* — [`ReloadReason::FileChanged`] is that
/// answer, and this is the only place it is known.
///
/// [`ReloadReason::FileChanged`]: crate::ReloadReason::FileChanged
pub(super) fn relevant_path<'event>(
    event: &'event Event,
    watched: &Watched,
) -> Option<&'event Path> {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return None;
    }

    event
        .paths
        .iter()
        .map(PathBuf::as_path)
        .find(|changed| is_ours(changed, watched))
}

fn is_ours(changed: &Path, watched: &Watched) -> bool {
    let explicit = watched.files.iter().any(|configured| {
        changed == configured || changed.ends_with(configured) || configured.ends_with(changed)
    });

    if explicit {
        return true;
    }

    // Only the searched directories are watched, so matching on the file name
    // alone is enough — and it catches a `config.toml` that did not exist when
    // the watcher started.
    if watched
        .search_name
        .as_deref()
        .is_some_and(|name| discovery::is_candidate(changed, name))
    {
        return true;
    }

    is_mount_marker(changed, watched)
}

/// Whether `changed` is the bookkeeping entry of an atomically remounted
/// directory holding one of our files.
///
/// Kubernetes updates a ConfigMap by writing a new timestamped directory and
/// swinging a `..data` symlink at it. The configuration file's own path never
/// receives an event — only `..data` and `..2026_08_09_12_00_00` do — so
/// matching on the file alone sees a ConfigMap update as silence. Entries
/// beginning with `..` are the kubelet's convention and effectively nothing
/// else's, which keeps this from firing on ordinary files.
fn is_mount_marker(changed: &Path, watched: &Watched) -> bool {
    let is_marker = changed
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".."));

    if !is_marker {
        return false;
    }

    let Some(directory) = changed.parent() else {
        return false;
    };

    let mut parents = watched.files.iter().filter_map(|file| file.parent());

    if parents.any(|parent| directory.ends_with(parent) || parent.ends_with(directory)) {
        return true;
    }

    watched
        .search_directories
        .iter()
        .any(|parent| directory.ends_with(parent) || parent.ends_with(directory))
}
