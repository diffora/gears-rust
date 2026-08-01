//! Typed repositories over the Foundation entities.
//!
//! Exactly one lands with the storage layer: the pin-frontier repository, whose
//! forward-only `advance` is a storage invariant rather than a caller
//! convention. The other nine tables get their repositories with the paths that
//! write them — a repository nothing calls is dead code, and dead code fails
//! CI here.

pub mod pin_frontier_repo;

pub use pin_frontier_repo::PinFrontierRepo;
