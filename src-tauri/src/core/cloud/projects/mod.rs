//! Cloud projects: backup and version history of project folders (docs/CLOUD_PROJECTS.md, a
//! copy of kinetra-cloud's; the contract wins). File bytes go straight to storage with
//! presigned URLs ([`r2`]); the API ([`api`]) sees only metadata. It's backup, not sync: the
//! server never pushes anything, nothing is merged, and the local project is always the primary
//! copy.
//!
//! # Rules that override everything (contract §0)
//! 1. Never interrupt graphics on air: no cloud error, quota, lapse or outage blocks saving,
//!    playout, rendering or opening a project, and nothing here opens a modal.
//! 2. Nothing the server says deletes or overwrites local files; restoring is a user action.
//! 3. Never merge: conflicting versions are both kept.
//! 4. File bytes go to storage only.
//! 5. Every downloaded file is SHA-256 checked before use.

pub mod api;
pub mod backoff;
pub mod backup;
pub mod gate;
pub mod history;
pub mod identity;
pub mod local;
pub mod paths;
pub mod r2;
pub mod restore;
pub mod scan;
pub mod schedule;
pub mod service;
pub mod wire;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_backup;
#[cfg(test)]
mod tests_identity;
#[cfg(test)]
mod tests_restore;
#[cfg(test)]
mod tests_service;
