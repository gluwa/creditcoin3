// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title IAttestCoinTreasuryVault
/// @notice Custody for attestcoin attestation-reward funds. The vault holds ATC and grants the
///         attestcoin precompile an ERC-20 allowance; the precompile pulls from the vault via
///         `transferFrom` when an attestor claims accrued rewards.
/// @dev Not an ERC-4626 vault: there are no shares, no yield accounting, and no deposit function.
///      Funding is passive — transfer tokens to the vault address and they are usable.
///
///      Roles:
///        - `owner`    — fund, defund, set the cap, set the spender, set the guardian, pause.
///        - `guardian` — `pauseRewardRedemptions()` only; cannot move funds or raise the cap.
///
///      The guardian is deliberately one-directional: there is no `unpause`. Resuming payouts is
///      `setAllowance`, which only the owner can call.
///
///      The ownership functions at the bottom of this interface are implemented by OpenZeppelin
///      `Ownable2Step`, not by hand; they are restated here so the whole callable surface is
///      visible to callers holding this interface type.
interface IAttestCoinTreasuryVault {
    /// @notice The precompile's spending cap was set to `amount`.
    event AllowanceSet(uint256 amount);
    /// @notice Reward redemptions were halted by driving the spender's allowance to zero.
    event RewardRedemptionsPaused(address indexed by);
    /// @notice The authorized spender changed. Any allowance held by the previous spender is revoked.
    event SpenderSet(address indexed previousSpender, address indexed newSpender);
    /// @notice The guardian changed. `newGuardian` may be the zero address (fast lever disabled).
    event GuardianSet(address indexed previousGuardian, address indexed newGuardian);
    /// @notice `amount` tokens were moved out of the vault to `to`.
    event Withdrawn(address indexed to, uint256 amount);

    /// @notice Set the spender's cap to an absolute value. Owner only.
    /// @dev Not additive: this replaces the current allowance rather than adding to it.
    /// @param amount The new allowance. The cap depletes as claims are paid and must be refreshed.
    function setAllowance(uint256 amount) external;

    /// @notice Drive the spender's allowance to zero, halting reward claims immediately.
    ///         Callable by the guardian or the owner. Moves no funds.
    /// @dev Intentionally has no counterpart; resume via `setAllowance`.
    function pauseRewardRedemptions() external;

    /// @notice Move tokens out of the vault. Owner only.
    function withdraw(address to, uint256 amount) external;

    /// @notice Replace the authorized spender. Owner only.
    /// @dev Revokes the previous spender's allowance and leaves the new spender at zero;
    ///      call `setAllowance` afterwards to authorize it.
    function setSpender(address newSpender) external;

    /// @notice Replace the guardian. Owner only. Pass the zero address to disable the fast lever.
    function setGuardian(address newGuardian) external;

    /// @notice The reward token. Immutable — a different token requires a redeployment.
    function token() external view returns (IERC20);

    /// @notice The authorized spender (the attestcoin precompile).
    function spender() external view returns (address);

    /// @notice The guardian, or the zero address if none.
    function guardian() external view returns (address);

    /// @notice What claims can actually draw right now: `min(balance, allowance)`.
    /// @dev Reads zero when paused, when unfunded, or when the cap is exhausted. Operators should
    ///      confirm this is non-zero after any configuration change.
    function availableToSpend() external view returns (uint256);

    // -------------------------------------------------------------------------
    // Ownership
    //
    // Supplied by OpenZeppelin `Ownable2Step` — declared here only so callers holding this
    // interface can reach them without a cast, and so the two-step handover is discoverable to
    // whoever drives it (governance, via `sudo.sudo_as`). The `OwnershipTransferStarted` and
    // `OwnershipTransferred` events are inherited from OpenZeppelin and are deliberately not
    // restated: redeclaring an inherited event is a compile error.
    // -------------------------------------------------------------------------

    /// @notice The current owner.
    function owner() external view returns (address);

    /// @notice The proposed owner, or the zero address if no transfer is pending.
    function pendingOwner() external view returns (address);

    /// @notice Propose a new owner. Owner only; `newOwner` must then call `acceptOwnership`.
    /// @dev Two-step by design: owner is the only role that can move funds, so a one-step
    ///      transfer to a mistyped address would permanently strand the treasury. Passing the
    ///      zero address cancels a pending transfer.
    function transferOwnership(address newOwner) external;

    /// @notice Accept a pending ownership transfer. Callable only by the pending owner.
    function acceptOwnership() external;
}
