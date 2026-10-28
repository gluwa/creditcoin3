// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @title IAttestorStash — stash-facing operations of `pallet-attestation`
/// @dev The precompile is deployed at address `0x0000000000000000000000000000000000000FD4`
///      (decimal 4052) in the Creditcoin 3 runtime.
///
/// Only stash-authored calls are exposed (`registerAttestor`,
/// `unregisterAttestor`, `chill`, `withdrawUnbonded`, `bondExtra`,
/// `unbondSurplus`). `attest` is authored by
/// the attestor itself, and operator-gated administration (set_max_attestors,
/// kick_active_attestor, etc.) is not routable through this precompile on
/// purpose — those must still be submitted as signed Substrate extrinsics.
///
/// The caller's EVM address is mapped to a Substrate `AccountId` via the
/// runtime's configured `AddressMapping` and used as the origin of the
/// dispatched pallet call. The dispatched call must therefore satisfy every
/// check that a normal signed extrinsic would (bond, ownership, existence,
/// etc.) and will revert the precompile call if the pallet returns an error.
/// @dev Attestor state returned by `getAttestor`.
struct AttestorInfo {
    bool exists;
    uint8 status;      // 0=Active, 1=Idle, 2=Waiting
    bytes32 stash;
    bool hasBlsKey;
}

/// @dev Ledger state returned by `getLedger`.
/// `withdrawable` is the sum of unlocking chunks whose unbonding era has
/// already elapsed — i.e. the amount `withdrawUnbonded` would actually
/// return right now. Distinguishes "funds unlocking but not ready" from
/// "funds ready to withdraw" in a single call.
struct LedgerInfo {
    bool exists;
    uint128 totalStaked;
    uint128 active;
    uint32 unlockingChunks;
    uint128 withdrawable;
}

interface IAttestorStash {
    /// @notice Emitted when a stash successfully registers an attestor.
    /// @param chainKey   The chain the attestor is being registered for.
    /// @param attestorId The 32-byte attestor identity (session/BLS key id).
    /// @param stash      The EVM address of the stash that authored the call.
    event AttestorRegistered(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash);

    /// @notice Emitted when a stash successfully unregisters an attestor.
    /// @param chainKey   The chain the attestor is being unregistered from.
    /// @param attestorId The 32-byte attestor identity.
    /// @param stash      The EVM address of the stash that authored the call.
    event AttestorUnregistered(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash);

    /// @notice Emitted when a stash successfully chills one of its attestors.
    /// @param chainKey   The chain the attestor is being chilled on.
    /// @param attestorId The 32-byte attestor identity.
    /// @param stash      The EVM address of the stash that authored the call.
    event AttestorChilled(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash);

    /// @notice Emitted when a stash successfully withdraws its fully-unbonded funds.
    /// @param stash The EVM address of the stash.
    event UnbondedWithdrawn(address indexed stash);

    /// @notice Emitted when a stash tops up its bond without registering an attestor.
    /// @param stash  The EVM address of the stash.
    /// @param amount Attest coin moved from the stash into the bond pool.
    event BondExtraAdded(address indexed stash, uint256 amount);

    /// @notice Emitted when a stash releases surplus bond into an unlocking chunk.
    /// @param stash  The EVM address of the stash.
    /// @param amount Attest coin moved from `active` into `unlocking`.
    event SurplusUnbonded(address indexed stash, uint256 amount);

    /// @notice Register a new attestor under the caller's stash for `chainKey`.
    /// @dev Calls `pallet_attestation::register_attestor`. Requires the stash
    ///      to have at least `MinBondRequirement` for the target chain and
    ///      `attestorId` to not already be registered.
    /// @param chainKey   The chain identifier.
    /// @param attestorId The 32-byte attestor identity.
    /// @return success `true` on successful dispatch.
    function registerAttestor(uint64 chainKey, bytes32 attestorId) external returns (bool success);

    /// @notice Unregister an attestor previously registered by the caller's stash.
    /// @dev Calls `pallet_attestation::unregister_attestor`. The caller must be
    ///      the stash that originally registered `attestorId` for `chainKey`.
    /// @param chainKey   The chain identifier.
    /// @param attestorId The 32-byte attestor identity.
    /// @return success `true` on successful dispatch.
    function unregisterAttestor(uint64 chainKey, bytes32 attestorId) external returns (bool success);

    /// @notice Chill one of the caller stash's attestors.
    /// @dev Calls `pallet_attestation::chill`. Although the underlying call is
    ///      named `chill`, it is authored by the stash — the pallet enforces
    ///      `attestor.stash == caller` — which is why it lives on this
    ///      stash-facing precompile.
    /// @param chainKey   The chain identifier.
    /// @param attestorId The 32-byte attestor identity.
    /// @return success `true` on successful dispatch.
    function chill(uint64 chainKey, bytes32 attestorId) external returns (bool success);

    /// @notice Withdraw the caller stash's fully-unbonded funds.
    /// @dev Calls `pallet_attestation::withdraw_unbonded`. No-op if there is
    ///      nothing eligible to withdraw yet (the pallet may still return an
    ///      error in that case, which will revert the precompile call).
    /// @return success `true` on successful dispatch.
    function withdrawUnbonded() external returns (bool success);

    /// @notice Top up the caller stash's bond by `amount` without registering another attestor.
    /// @dev Calls `pallet_attestation::bond_extra`. Moves `amount` from the stash's liquid
    ///      attest-coin (`pallet-assets`) balance into the bond pool and credits `active`.
    ///      Reverts if the caller has no ledger (register an attestor first) or if `amount`
    ///      exceeds the stash's liquid balance. `amount` must fit in 128 bits.
    /// @param amount Attest coin to bond, in the asset's smallest unit.
    /// @return success `true` on successful dispatch.
    function bondExtra(uint256 amount) external returns (bool success);

    /// @notice Release `amount` of surplus bond into an unlocking chunk.
    /// @dev Calls `pallet_attestation::unbond_surplus` — the inverse of `bondExtra`, and the only
    ///      way to release bond sitting above the stash's aggregate requirement (reachable after
    ///      governance *lowers* a chain's `MinBondRequirement` below what the stash bonded at
    ///      registration). `active` may only be reduced to the sum of `MinBondRequirement` across
    ///      the stash's still-registered attestors, never below; with none registered that sum is
    ///      zero, so the whole remaining bond is releasable. Call `withdrawUnbonded` once the
    ///      chunk's unbonding era has elapsed. `amount` must fit in 128 bits.
    /// @param amount Attest coin to move from `active` into `unlocking`.
    /// @return success `true` on successful dispatch.
    function unbondSurplus(uint256 amount) external returns (bool success);

    /// @notice Returns attestor state for the given chain and attestor id.
    /// @param chainKey   The chain identifier.
    /// @param attestorId The 32-byte attestor identity.
    /// @return info AttestorInfo struct (exists=false if not registered).
    function getAttestor(uint64 chainKey, bytes32 attestorId) external view returns (AttestorInfo memory info);

    /// @notice Returns true if the attestor is in the active set for `chainKey`.
    /// @param chainKey   The chain identifier.
    /// @param attestorId The 32-byte attestor identity.
    /// @return active `true` if in the active set.
    function isActiveAttestor(uint64 chainKey, bytes32 attestorId) external view returns (bool active);

    /// @notice Returns the number of registered attestors for `chainKey`.
    /// @param chainKey The chain identifier.
    /// @return count Number of registered attestors.
    function getAttestorsCount(uint64 chainKey) external view returns (uint32 count);

    /// @notice Returns ledger info for the given stash account.
    /// @param stash The 32-byte stash account id.
    /// @return info LedgerInfo struct (exists=false if no ledger).
    function getLedger(bytes32 stash) external view returns (LedgerInfo memory info);

    /// @notice Returns the minimum bond requirement for `chainKey`.
    /// @param chainKey The chain identifier.
    /// @return minBond Minimum bond amount.
    function getMinBondRequirement(uint64 chainKey) external view returns (uint128 minBond);
}
