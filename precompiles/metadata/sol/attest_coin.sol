// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @dev Attest-coin precompile (`runtime` `hash(4052)`).
address constant ATTEST_COIN_PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000fd4;

/// @dev Attest-coin precompile at `0x0000000000000000000000000000000000000fd4` (runtime hash 4052).
interface IAttestCoinPrecompile {
    /// @notice Returns the accrued reward points (1e18 precision) for the given Substrate stash account.
    function accrued(bytes32 stash) external view returns (uint256);

    /// @notice Claim accrued reward points as ERC-20 tokens to the caller's EVM address.
    /// @dev `evmRecipient` must equal `msg.sender`.
    ///      `sigHi`/`sigLo` are the sr25519 signature (32+32 bytes) over the runtime-defined
    ///      message (see `pallet_attest_coin_rewards::Pallet::claim_signing_message`).
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
}
