//! `MessageVote` — the envelope attestors gossip on `{chain_key}/message-votes/v1`.
//!
//! The canonical definition lives in the shared [`write_ability`] crate so the attestor (which
//! signs and publishes votes) and this relayer (which decodes and counts them) stay byte-compatible
//! on the wire. See [`write_ability::envelope`].

pub use write_ability::envelope::*;
