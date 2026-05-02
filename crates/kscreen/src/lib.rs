//! KDE `KScreen` backend using `kscreen-doctor`

// Cross-target dependency trees currently pull windows-sys through independent
// upstream paths that cannot be unified from this crate.
#![allow(clippy::multiple_crate_versions)]

pub mod backend;
