//! Task entrypoints. Each task is just an `async fn run(shared, …)` — there is no `Worker`
//! trait, no separate OS thread, no per-task tokio runtime. They are all driven by the main
//! runtime started in `main.rs` and supervised by the `JoinSet` in `lib.rs`.

pub mod api;
pub mod p2p;
pub mod production;
pub mod runtime_updater;
pub mod validation;
