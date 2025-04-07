use crate::{AttestationCheckpoint, ChainKey, Digest, SignedAttestation};
use parity_scale_codec::{Codec, Decode};

pub trait CheckpointProvider {
    fn get_checkpoint(chain_key: ChainKey, digest: Digest) -> Option<AttestationCheckpoint>;

    fn get_checkpoint_interval(chain_key: ChainKey) -> u64;

    fn get_last_checkpoint_number(chain_key: ChainKey) -> Option<u64>;
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

    fn get_attestation_interval(chain_key: ChainKey) -> u64;
}
