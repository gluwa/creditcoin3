use crate::{AttestationCheckpoint, ChainKey, Digest};

pub trait CheckpointProvider {
    fn get_checkpoint(chain_key: ChainKey, digest: Digest) -> Option<AttestationCheckpoint>;
}
