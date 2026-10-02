//! REST doors under `/bss-approvals/v1`.

mod dto;
mod handlers;
mod routes;

#[cfg(test)]
#[path = "doors_tests.rs"]
mod doors_tests;

pub(crate) use routes::router;
