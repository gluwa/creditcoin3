// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @dev Attest-coin precompile (`runtime` `hash(4053)`).
address constant ATTEST_COIN_PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000fd5;

/// @dev Attest-coin precompile at `0x0000000000000000000000000000000000000fd5` (runtime hash 4053).
interface IAttestCoinPrecompile {
    /// @notice Returns the accrued reward points (1e18 precision) for the given Substrate stash account.
    function accrued(bytes32 stash) external view returns (uint256);

    /// @notice Claim accrued reward points as ERC-20 tokens to the caller's EVM address.
    /// @dev `evmRecipient` must equal `msg.sender`.
    ///      Authorized either way:
    ///        - If `stash` is the runtime `AddressMapping` image of `msg.sender` — the identity of
    ///          every attestor registered via the attestor-stash precompile — this call is
    ///          self-authorizing and `sigHi`/`sigLo` are ignored (pass zero). Such a stash is a
    ///          blake2 hash with no sr25519 key, so no signature over it can ever exist.
    ///        - Otherwise `sigHi`/`sigLo` must be the stash's sr25519 signature (32+32 bytes) over
    ///          the runtime-defined message (see
    ///          `pallet_attest_coin_rewards::Pallet::claim_signing_message`).
    ///      Either way the claim only ever spends the named stash's accrual, paid to `msg.sender`,
    ///      and `ClaimNonce` prevents replay.
    function claim(
        bytes32 stash,
        uint256 nonce,
        uint256 chainKey,
        uint256 amount,
        address evmRecipient,
        bytes32 sigHi,
        bytes32 sigLo
    ) external;

    /// @notice Deposit ERC-20 attest-coin tokens into the Substrate `pallet-assets` balance of the
    ///         caller's mapped Substrate account.
    /// @dev The caller must first call `approve(precompile_address, amount)` on the ERC-20 contract.
    ///      The precompile will then call `transferFrom(caller, precompile, amount)` on the ERC-20
    ///      and mint the equivalent amount into the caller's Substrate account.
    function deposit(uint256 amount) external;

    /// @notice Same as `deposit` but mints to an explicit 32-byte Substrate `AccountId` instead of
    ///         the caller's mapped account.
    /// @dev The caller must first call `approve(precompile_address, amount)` on the ERC-20 contract.
    ///      `beneficiary` must not be the zero bytes32.
    function depositTo(uint256 amount, bytes32 beneficiary) external;

    /// @notice Burn liquid attest coin from the caller's mapped Substrate `pallet-assets` balance and
    ///         receive the same amount of ERC-20 on the caller's EVM address (inverse of `deposit`).
    /// @dev Requires the attest-coin asset **admin** to be the precompile account (runtime migration).
    function withdraw(uint256 amount) external;

    /// @notice Burn liquid attest coin from an explicit Substrate `stash` and send the same amount of
    ///         ERC-20 to the caller. The sr25519-authorized inverse of `depositTo`.
    /// @dev `withdraw` can only burn from the caller's own mapped account, so an sr25519 stash that
    ///      received attest coin via `depositTo` needs this entry to get back out to ERC-20 without
    ///      controlling a second EVM-space key.
    ///      `evmRecipient` must equal `msg.sender`. `sigHi`/`sigLo` are the stash's sr25519
    ///      signature (32+32 bytes) over `pallet_attest_coin_rewards::Pallet::withdraw_signing_message`,
    ///      which binds the genesis hash, stash, `nonce`, `amount` and `evmRecipient`. Read the next
    ///      expected nonce from `attestCoinRewards.withdrawNonce(stash)`; it is a counter separate
    ///      from the claim nonce. The nonce is consumed before any token movement and rolled back if
    ///      that movement fails, so a failed attempt does not burn the signature.
    ///      Requires the attest-coin asset **admin** to be the precompile account.
    function withdrawFrom(
        bytes32 stash,
        uint256 nonce,
        uint256 amount,
        address evmRecipient,
        bytes32 sigHi,
        bytes32 sigLo
    ) external;
}
