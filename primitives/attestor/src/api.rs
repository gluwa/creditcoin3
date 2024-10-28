use parity_scale_codec::{Codec, Decode};

use crate::{AttestorStatus, ChainId, ChainKey, Digest, SignedAttestation};

use super::BlsPublicKey;

sp_api::decl_runtime_apis! {
    pub trait AttestorApi<H, AccountId>
        where
                AccountId: Codec + Decode,
                H: Decode,
    {
        fn is_attestor(chain_id:ChainId, attestor: &AccountId) -> bool;

        fn comittee_set_size(chain_id: ChainId) -> u32;

        fn working_set_size(chain_id: ChainId) -> u32;

        fn last_digest(chain_id: ChainId) -> Option<Digest>;

        fn get(chain_id: ChainId, digest: Digest) -> Option<SignedAttestation<H, AccountId>>;

        fn contains_digest(chain_id: ChainId, digest: Digest) -> bool;

        fn attestor_bls_pubkey(chain_id:ChainId, attestor: &AccountId) -> Option<BlsPublicKey>;

        fn chain_attestation_interval(chain_key: ChainKey) -> Option<u64>;

        fn attestor_status(chain_id:ChainId, attestor: &AccountId) -> Option<AttestorStatus>;
    }
}
