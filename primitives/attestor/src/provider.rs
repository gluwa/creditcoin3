use crate::{AttestationCheckpoint, ChainKey, PalletDigest, SignedAttestation};
use parity_scale_codec::{Codec, Decode};

pub trait CheckpointProvider {
    fn get_checkpoint(chain_key: ChainKey, digest: PalletDigest) -> Option<AttestationCheckpoint>;
}

pub trait AttestationProvider<H, AccountId>
where
    AccountId: Codec + Decode,
    H: Decode,
{
    fn get_attestation(
        chain_key: ChainKey,
        digest: PalletDigest,
    ) -> Option<SignedAttestation<H, AccountId>>;
}
