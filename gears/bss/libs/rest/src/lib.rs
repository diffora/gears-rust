//! Shared HTTP helpers for the BSS gears.
//!
//! [`conditional_get`] turns a successful read into a weak `ETag` over the JSON
//! bytes actually served, and answers `304` when `If-None-Match` matches.
//!
//! [`actor_names`] reads the names of the actors one response shows, through
//! Account Management, in one bounded lookup per response.

pub mod actor_names;
pub mod conditional_get;
