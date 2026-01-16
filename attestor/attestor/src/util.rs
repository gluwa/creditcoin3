use crate::prelude::*;

pub(crate) fn next_multiple_of(
    attestation_interval: std::num::NonZero<common::types::Height>,
    target_height: common::types::Height,
) -> common::types::Height {
    // NOTE: EDGE CASE
    //
    // After an attestation interval changes, it is possible for the attestor to observe a new
    // finalized attestation on the execution chain which was:
    //
    // - Produced BEFORE the interval change.
    // - Appears as finalized AFTER the interval change.
    //
    // This can happen if the interval change extrinsic is submitted midway during attestation
    // validation, or due to network latency if the interval change gets noticed before the new
    // finalized attestation.
    //
    // This avoids the follow edge case where we observe:
    //
    //  1. Submit attestation 1
    //  2. Attestation 1 is received by the runtime
    //  3. Finalized attestation 1
    //  4. Target height is now 1
    //  5. Submit attestation 2
    //  6. Attestation 2 is received by the runtime
    //  7. Interval change to 10
    //  8. Target height is now 10
    //  9. Finalized attestation 2 (validated before the interval change)
    // 10. Target height is now 12
    //
    // This could happen if we naively increment the target height by the attestation interval.
    //
    // Since the attestation pool expects an EXACT target height, and since attestation 12 cannot be
    // produced at an interval of 10, this would lead to a stall.
    let attestation_interval = attestation_interval.get();
    target_height.saturating_add(attestation_interval - (target_height % attestation_interval))
}
