//! One approval-unit shape for the BSS gears (spec §6, §2.2).
//!
//! A gear owns four tables built from [`ddl`], implements [`store::Store`] over
//! them and [`subject::ApprovalSubject`] once per kind of change; [`engine::Engine`]
//! drives submit, approve, reject and withdraw through those two traits inside
//! the gear's own transaction. Nothing here opens a connection or names a table.
pub mod ddl;
pub mod engine;
pub mod hash;
pub mod model;
pub mod rules;
pub mod store;
pub mod subject;

// pub use engine::{ApproveOutcome, Engine, SubmitRequest, Submitted};
// pub use model::{ApprovalError, Decision, ItemRef, Policy, Unit, UnitState, Verdict};
// pub use store::Store;
// pub use subject::ApprovalSubject;
