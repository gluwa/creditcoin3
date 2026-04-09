//! Storage migrations for pallet-attestation.
//!
//! Migrates `SignedAttestation.continuity_proof` from `AttestationFragmentSerializable`
//! (blocks with block_number, root, prev_digest, digest) to `ContinuityProof`
//! (lower_endpoint_digest, roots).

use frame_support::{
    pallet_prelude::*,
    traits::{GetStorageVersion, OnRuntimeUpgrade},
    weights::Weight,
};
#[cfg(feature = "try-runtime")]
use parity_scale_codec::DecodeAll;
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
use sp_runtime::AccountId32;
#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
use sp_std::{marker::PhantomData, vec::Vec};

use crate::pallet::{Attestations, Config, Pallet};
use attestor_primitives::{
    block::ContinuityProof, AttestationData, BlsSignature, Digest, SignedAttestation,
};

/// Old continuity proof format: blocks with full block data.
#[derive(Decode, Encode)]
struct OldBlockSerializable {
    block_number: u64,
    root: H256,
    prev_digest: H256,
    digest: H256,
}

/// Old attestation fragment: Vec of blocks.
#[derive(Decode, Encode)]
struct OldAttestationFragment {
    blocks: Vec<OldBlockSerializable>,
}

/// Old SignedAttestation with AttestationFragmentSerializable continuity_proof.
#[derive(Decode, Encode)]
struct OldSignedAttestation {
    attestation: AttestationData<Digest>,
    signature: BlsSignature,
    attestors: Vec<AccountId32>,
    continuity_proof: OldAttestationFragment,
}

fn convert_old_to_new(old: OldSignedAttestation) -> SignedAttestation<Digest, AccountId32> {
    let continuity_proof = if old.continuity_proof.blocks.is_empty() {
        ContinuityProof::default()
    } else {
        let lower_endpoint_digest = old.continuity_proof.blocks[0].prev_digest;
        let roots: Vec<H256> = old.continuity_proof.blocks.iter().map(|b| b.root).collect();
        ContinuityProof::new(lower_endpoint_digest, roots)
    };

    SignedAttestation {
        attestation: old.attestation,
        signature: old.signature,
        attestors: old.attestors,
        continuity_proof,
    }
}

/// Migration V0 -> V1: AttestationFragmentSerializable -> ContinuityProof
pub struct MigrateAttestationContinuityProofV0ToV1<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateAttestationContinuityProofV0ToV1<T>
where
    T::Hash: From<H256>,
    T::AccountId: From<AccountId32>,
{
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!(
                "Attestation migration: already at v1 or above (on_chain={on_chain:?}), skipping"
            );
            return T::DbWeight::get().reads(1);
        }

        log::info!("Attestation migration: upgrading from {on_chain:?} to {target:?}");

        let mut count = 0u64;

        Attestations::<T>::translate::<OldSignedAttestation, _>(|_chain_key, _digest, old| {
            count += 1;
            let migrated = convert_old_to_new(old);
            Some(SignedAttestation {
                attestation: AttestationData {
                    chain_key: migrated.attestation.chain_key,
                    header_number: migrated.attestation.header_number,
                    header_hash: migrated.attestation.header_hash.into(),
                    root: migrated.attestation.root.into(),
                    prev_digest: migrated.attestation.prev_digest,
                },
                signature: migrated.signature,
                attestors: migrated.attestors.into_iter().map(|a| a.into()).collect(),
                continuity_proof: migrated.continuity_proof,
            })
        });

        target.put::<Pallet<T>>();

        log::info!(
            "Migrated {count} attestations from AttestationFragmentSerializable to ContinuityProof"
        );

        T::DbWeight::get().reads_writes(count + 1, count + 1)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!("pre_upgrade: already at v1 or above (on_chain={on_chain:?}), skipping");
            return Ok((0u64, false).encode());
        }

        let count = Attestations::<T>::iter_values().count() as u64;

        log::info!("pre_upgrade: found {count} attestations to migrate");

        Ok((count, true).encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
        let (pre_count, should_run): (u64, bool) = DecodeAll::decode_all(&mut &state[..])
            .map_err(|_| "failed to decode pre_upgrade state")?;

        if !should_run {
            log::info!("post_upgrade: migration was skipped, nothing to verify");
            return Ok(());
        }

        let on_chain = Pallet::<T>::on_chain_storage_version();
        let current = Pallet::<T>::in_code_storage_version();
        ensure!(
            on_chain == current,
            "post_upgrade: storage version not updated (on_chain={on_chain:?}, current={current:?})"
        );

        let post_count = Attestations::<T>::iter_values().count() as u64;

        ensure!(
            pre_count == post_count,
            "post_upgrade: attestation count mismatch (pre={pre_count}, post={post_count})"
        );

        // Verify continuity proof invariants for each migrated entry:
        // - empty proof: both fields should be default (no roots, zero digest)
        // - non-empty proof: must have a non-zero lower_endpoint_digest
        //   (sourced from the first old block's prev_digest)
        for (_chain_key, _digest, attestation) in Attestations::<T>::iter() {
            let proof = &attestation.continuity_proof;
            if proof.roots.is_empty() {
                ensure!(
                    proof.lower_endpoint_digest == H256::zero(),
                    "post_upgrade: empty proof has non-zero lower_endpoint_digest"
                );
            } else {
                ensure!(
                    proof.lower_endpoint_digest != H256::zero(),
                    "post_upgrade: non-empty proof has zero lower_endpoint_digest"
                );
            }
        }

        log::info!("post_upgrade: verified {post_count} attestations migrated successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_empty_fragment() {
        let old = OldSignedAttestation {
            attestation: AttestationData {
                chain_key: 1,
                header_number: 100,
                header_hash: H256::default(),
                root: H256::default(),
                prev_digest: None,
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof: OldAttestationFragment { blocks: vec![] },
        };
        let new = convert_old_to_new(old);

        // Verify attestation fields are preserved
        assert_eq!(new.attestation.chain_key, 1);
        assert_eq!(new.attestation.header_number, 100);
        assert_eq!(new.attestation.header_hash, H256::default());
        assert_eq!(new.attestation.root, H256::default());
        assert_eq!(new.attestation.prev_digest, None);
        assert_eq!(new.signature, [0; 96]);
        assert!(new.attestors.is_empty());

        // Verify empty continuity proof
        assert!(new.continuity_proof.is_empty());
        assert_eq!(new.continuity_proof.tail_prev_digest(), None);
    }

    #[test]
    fn test_convert_single_block() {
        let prev = H256::from_low_u64_be(1);
        let root = H256::from_low_u64_be(2);
        let digest = H256::from_low_u64_be(3);
        let old = OldSignedAttestation {
            attestation: AttestationData {
                chain_key: 1,
                header_number: 100,
                header_hash: H256::default(),
                root: H256::default(),
                prev_digest: None,
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof: OldAttestationFragment {
                blocks: vec![OldBlockSerializable {
                    block_number: 99,
                    root,
                    prev_digest: prev,
                    digest,
                }],
            },
        };
        let new = convert_old_to_new(old);

        // Verify attestation fields are preserved
        assert_eq!(new.attestation.chain_key, 1);
        assert_eq!(new.attestation.header_number, 100);
        assert_eq!(new.attestation.header_hash, H256::default());
        assert_eq!(new.attestation.root, H256::default());
        assert_eq!(new.attestation.prev_digest, None);
        assert_eq!(new.signature, [0; 96]);
        assert!(new.attestors.is_empty());

        // Verify continuity proof structure
        assert_eq!(new.continuity_proof.len(), 1);
        assert_eq!(new.continuity_proof.lower_endpoint_digest, prev);
        assert_eq!(new.continuity_proof.roots[0], root);

        // Verify tail_prev_digest returns the lower endpoint
        assert_eq!(new.continuity_proof.tail_prev_digest(), Some(prev));

        // Verify compute_continuity_digest matches the expected digest chain
        // start_block_number = header_number - len = 100 - 1 = 99
        let expected_digest = ContinuityProof::hash_payload(&99, &root, &prev);
        assert_eq!(
            new.continuity_proof.compute_continuity_digest(99),
            expected_digest
        );
    }
}
