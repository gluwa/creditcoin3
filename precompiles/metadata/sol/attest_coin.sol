// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @dev Attest-coin precompile at `0x0000000000000000000000000000000000000fd4` (runtime hash 4052).
interface IAttestCoinPrecompile {
    function accrued(bytes32 stash) external view returns (uint256);

    /// @notice `evmRecipient` must equal `msg.sender`. Signature is sr25519 over the runtime-defined
    ///         message (see `pallet_attest_coin_rewards::Pallet::claim_signing_message`).
    function claim(
        bytes32 stash,
        uint256 nonce,
        uint256 chainKey,
        uint256 amount,
        address evmRecipient,
        bytes32 sigHi,
        bytes32 sigLo
    ) external;
}
