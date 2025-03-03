use crate::{AttestationCheckpoint, ChainKey, Digest, SignedAttestation};
use parity_scale_codec::{Codec, Decode};

pub trait CheckpointProvider {
    fn get_checkpoint(chain_key: ChainKey, digest: Digest) -> Option<AttestationCheckpoint>;
}

pub trait AttestationProvider<H, AccountId>
where
    AccountId: Codec + Decode,
    H: Decode,
{
    fn get_attestation(
        chain_key: ChainKey,
        digest: Digest,
    ) -> Option<SignedAttestation<H, AccountId>>;
}
