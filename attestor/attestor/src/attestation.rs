//! Configuration options around attestation production.

use crate::prelude::*;

#[derive(Debug, attestor_macro::Builder)]
/// Attestation production options
pub struct Config {
    /// **Optional** forced interval at which attestations are produced. By default this value is
    /// fetched from on-chain storage -this options overrides it.
    pub attestation_interval: Option<std::num::NonZero<common::types::Height>>,

    /// **Optional** forced interval at which checkpoints are stored. By default this value is
    /// fetched from on-chain storage -this options overrides it.
    pub checkpoint_interval: Option<std::num::NonZero<common::types::Height>>,

    /// **Optional** forced attestation start height. By default this value is fetched from
    /// on-chain storage -this option overrides it.
    pub start_height: Option<common::types::Height>,
}
