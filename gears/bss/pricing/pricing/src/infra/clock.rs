//! Commercial time uses the existing reference/recovery clock, never a second wall-time source.
//! The inherited default jitter hook is irrelevant to commercial operations.
pub use super::reference_work::{Clock, WallClock as SystemClock};
