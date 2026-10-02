//! Shared HTTP helpers for the BSS gears.
//!
//! [`conditional_get`] turns a successful read into a weak `ETag` over the JSON
//! bytes actually served, and answers `304` when `If-None-Match` matches.

pub mod conditional_get;
