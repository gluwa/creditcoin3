//! Data [`Stream`]s used to react to [source chain] and [execution chain] progress.
//!
//! [`Stream`]: futures::Stream
//! [source chain]: attestation
//! [execution chain]: cc3

pub mod attestation;
pub mod cc3;

#[derive(Debug, attestor_macro::Builder)]
pub struct Config {
    pub(crate) url_eth: url::Url,
    pub(crate) url_cc3: url::Url,
    pub(crate) secret: bip39::Mnemonic,
}
