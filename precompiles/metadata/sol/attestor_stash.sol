// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The Attestor Stash precompile address (hash(4052) == 0xFD4)
address constant ATTESTOR_STASH_ADDRESS = 0x0000000000000000000000000000000000000fd4;

AttestorStash constant ATTESTOR_STASH_CONTRACT = AttestorStash(ATTESTOR_STASH_ADDRESS);

/// @dev Attestor state returned by `getAttestor`.
struct AttestorInfo {
    bool exists;
    uint8 status; // 0=Active, 1=Idle, 2=Waiting
    bytes32 stash;
    bool hasBlsKey;
}

/// @dev Ledger state returned by `getLedger`.
/// `withdrawable` is the sum of unlocking chunks whose unbonding era has
/// already elapsed — i.e. the amount `withdrawUnbonded` would actually
/// return right now.
struct LedgerInfo {
    bool exists;
    uint128 totalStaked;
    uint128 active;
    uint32 unlockingChunks;
    uint128 withdrawable;
}

/// @title AttestorStash — stash-facing operations of `pallet-attestation`
/// @notice Only stash-authored calls are exposed here (`registerAttestor`,
///         `unregisterAttestor`, `chill`, `withdrawUnbonded`). `attest` is
///         authored by the attestor account and is intentionally *not*
///         exposed through this precompile; operator-gated calls are not
///         exposed either.
/// @dev The caller's EVM address is mapped to a Substrate `AccountId` via the
///      runtime's configured `AddressMapping` and used as the origin of the
///      dispatched pallet call. If the dispatched call returns an error, the
///      precompile reverts.
interface AttestorStash {
    /// @notice Emitted when a stash successfully registers an attestor.
    event AttestorRegistered(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash);

    /// @notice Emitted when a stash successfully unregisters an attestor.
    event AttestorUnregistered(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash);

    /// @notice Emitted when a stash successfully chills one of its attestors.
    event AttestorChilled(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash);

    /// @notice Emitted when a stash successfully withdraws fully-unbonded funds.
    event UnbondedWithdrawn(address indexed stash);

    /// @notice Register a new attestor under the caller's stash for `chainKey`.
    /// @dev Mirrors `pallet_attestation::register_attestor`. Requires the
    ///      stash to have at least `MinBondRequirement` for the target chain
    ///      and `attestorId` to not already be registered.
    function registerAttestor(uint64 chainKey, bytes32 attestorId) external returns (bool);

    /// @notice Unregister an attestor previously registered by the caller's stash.
    /// @dev Mirrors `pallet_attestation::unregister_attestor`.
    function unregisterAttestor(uint64 chainKey, bytes32 attestorId) external returns (bool);

    /// @notice Chill one of the caller stash's attestors.
    /// @dev Mirrors `pallet_attestation::chill`. Although the extrinsic is
    ///      named `chill`, it is authored by the stash (the pallet enforces
    ///      `attestor.stash == caller`).
    function chill(uint64 chainKey, bytes32 attestorId) external returns (bool);

    /// @notice Withdraw the caller stash's fully-unbonded funds.
    /// @dev Mirrors `pallet_attestation::withdraw_unbonded`.
    function withdrawUnbonded() external returns (bool);

    /// @notice Returns attestor state for the given chain and attestor id.
    /// @return info `AttestorInfo` struct (`exists == false` if not registered).
    function getAttestor(uint64 chainKey, bytes32 attestorId) external view returns (AttestorInfo memory info);

    /// @notice Returns true if the attestor is in the active set for `chainKey`.
    function isActiveAttestor(uint64 chainKey, bytes32 attestorId) external view returns (bool active);

    /// @notice Returns the number of registered attestors for `chainKey`.
    function getAttestorsCount(uint64 chainKey) external view returns (uint32 count);

    /// @notice Returns ledger info for the given stash account.
    /// @param stash The **hashed** `AccountId32` produced by the runtime's `AddressMapping`
    ///        from the EVM address — NOT the raw 20-byte EVM address zero-padded to 32
    ///        bytes. EVM consumers emitting events tied to their own `msg.sender` should
    ///        prefer `getLedgerByAddress` or `getCallerLedger` to avoid the silently-empty
    ///        ledger foot-gun.
    /// @return info `LedgerInfo` struct (`exists == false` if no ledger).
    function getLedger(bytes32 stash) external view returns (LedgerInfo memory info);

    /// @notice Returns ledger info for the EVM `addr`.
    /// @dev Applies the runtime's `AddressMapping` internally so EVM-side consumers can
    ///      look up their own ledger using the same identifier (the EVM `address`) that
    ///      events and state-changing calls in this precompile already use.
    /// @return info `LedgerInfo` struct (`exists == false` if no ledger).
    function getLedgerByAddress(address addr) external view returns (LedgerInfo memory info);

    /// @notice Returns ledger info for `msg.sender`.
    /// @dev Convenience entry equivalent to `getLedgerByAddress(msg.sender)`.
    /// @return info `LedgerInfo` struct (`exists == false` if no ledger).
    function getCallerLedger() external view returns (LedgerInfo memory info);

    /// @notice Returns the minimum bond requirement for `chainKey`.
    function getMinBondRequirement(uint64 chainKey) external view returns (uint128 minBond);
}
