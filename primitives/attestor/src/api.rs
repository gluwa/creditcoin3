use parity_scale_codec::Codec;
use sp_core::H256;

use crate::ChainId;

use super::BlsPublicKey;

pub type Digest = H256;

sp_api::decl_runtime_apis! {
    pub trait AttestorApi<AccountId>
        where AccountId: Codec
    {
        fn is_attestor(attestor: &AccountId) -> bool;

        fn comittee_set_size() -> u32;

        fn last_digest(chain_id: ChainId) -> Option<Digest>;

        fn contains_digest(chain_id: ChainId, digest: Digest) -> bool;

        fn attestor_bls_pubkey(attestor: &AccountId) -> Option<BlsPublicKey>;

        fn chain_attestation_interval(chain_id: ChainId) -> Option<u64>;
    }
}
