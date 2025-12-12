//! Chain listeners are an abstraction layer responsible for retrieving and handling data from a
//! chain. They are used in the [production worker] to drive attestation production and by the
//! [p2p worker] to guarantee liveness.
//!
//! [production worker]: crate::worker::production
//! [p2p worker]: crate::worker::p2p

pub mod cc3;
pub mod eth;
pub mod rebroadcast;

use crate::prelude::*;

#[derive(Debug)]
struct Catchup {
    start: common::types::Height,
    stop: common::types::Height,
}
