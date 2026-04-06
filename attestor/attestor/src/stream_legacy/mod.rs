//! Data [`Stream`]s used to react to [source chain] and [execution chain] progress.
//!
//! # What is the difference between a channel and a stream?
//!
//! A channel is a passive transport mechanism — it does not schedule or delay emissions. A stream
//! on the other hand has full control over the way in which it is paused/resumed, allowing for the
//! implementation of much more complex and fine-grained waiting logic.
//!
//! Under the hood all asynchronous code in Rust ends up calling some low-level manual [`Future`]
//! implementation. Streams work at that level to allow full control over the async execution model
//! of your code. In contrast, channels are much higher-level constructs which do not allow for
//! manual fine-tuning.
//!
//! [`Stream`]: futures::Stream
//! [source chain]: attestation
//! [execution chain]: cc3
//! [`Future`]: std::future::Future

pub mod cc3;

#[derive(Debug, builder::Builder)]
pub struct Config {
    pub(crate) url_eth: url::Url,
    pub(crate) url_cc3: url::Url,
}
