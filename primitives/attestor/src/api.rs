use parity_scale_codec::{Codec, Decode};
use sp_std::vec::Vec;

use crate::{AttestorStatus, ChainKey, Digest, PalletDigest, SignedAttestation};

use super::BlsPublicKey;

sp_api::decl_runtime_apis! {
    pub trait AttestorApi<H, AccountId>
        where
            AccountId: Codec + Decode,
            H: Decode,
    {
        fn is_attestor(chain_key :ChainKey, attestor: &AccountId) -> bool;

        fn target_sample_size(chain_key: ChainKey) -> u32;

        fn working_set_size(chain_key: ChainKey) -> u32;

        fn last_digest(chain_key: ChainKey) -> Option<PalletDigest>;

        fn get(chain_key: ChainKey, digest: PalletDigest) -> Option<SignedAttestation<H, AccountId>>;

        fn contains_digest(chain_key: ChainKey, digest: PalletDigest) -> bool;

        fn attestor_bls_pubkey(chain_key: ChainKey, attestor: &AccountId) -> Option<BlsPublicKey>;

        fn chain_attestation_interval(chain_key: ChainKey) -> u64;

        fn attestor_status(chain_key: ChainKey, attestor: &AccountId) -> Option<AttestorStatus>;

        fn active_attestor_set(chain_key: ChainKey) -> Vec<AccountId>;

        fn attestation_checkpoint_interval(chain_key: ChainKey) -> u32;
    }
}
