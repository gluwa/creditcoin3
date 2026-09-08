// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

import {IAttestCoinTreasuryVault} from "./IAttestCoinTreasuryVault.sol";

/// @title AttestCoinTreasuryVault
/// @notice Holds attestcoin attestation-reward funds and grants the attestcoin precompile an
///         allowance to spend them. See `treasury-design.md`.
/// @dev The vault contains no reward logic: it does not know what an attestor is, does not read
///      Substrate state, and never calls into the precompile. The dependency is one-directional —
///      the precompile pulls from the vault, and the vault knows only a spender address and a cap.
///
///      Ownership is two-step via OpenZeppelin `Ownable2Step`: owner is the only role that can move
///      funds, so a one-step transfer to a mistyped address would permanently strand the treasury.
contract AttestCoinTreasuryVault is IAttestCoinTreasuryVault, Ownable2Step {
    using SafeERC20 for IERC20;

    /// @notice The reward token, fixed at deployment.
    /// @dev Immutable by design. It must equal the value the attestcoin rewards pallet stores in
    ///      `AttestCoinErc20`; a mismatch stops payouts rather than misdirecting them, because the
    ///      precompile pulls against the *pallet's* token, where this vault has granted no
    ///      allowance. Correcting a mismatch is a redeployment, not a setter.
    // forge-lint: disable-next-line(screaming-snake-case-immutable)
    IERC20 public immutable token;

    /// @inheritdoc IAttestCoinTreasuryVault
    address public override spender;

    /// @inheritdoc IAttestCoinTreasuryVault
    address public override guardian;

    error NotGuardianOrOwner();
    error ZeroAddress();
    error ZeroAmount();

    /// @param token_ The reward ERC-20. Must match the rewards pallet's `AttestCoinErc20`.
    /// @param owner_ Initial owner. Reachable by root via `sudo.sudo_as`, since `pallet_evm`'s
    ///        `CallOrigin` is `EnsureAddressTruncated` — root can source an EVM call from any
    ///        address by choosing an `AccountId` whose first 20 bytes match it.
    /// @param spender_ The attestcoin precompile (`0x0000000000000000000000000000000000000fd5`).
    /// @param guardian_ Pause-only role. May be the zero address to disable the fast lever.
    /// @dev No allowance is granted at construction: funding and authorizing are deliberately
    ///      separate steps, so call `setAllowance` once the vault is funded.
    constructor(IERC20 token_, address owner_, address spender_, address guardian_)
        Ownable(owner_)
    {
        if (address(token_) == address(0) || spender_ == address(0)) {
            revert ZeroAddress();
        }
        token = token_;
        spender = spender_;
        guardian = guardian_;

        emit SpenderSet(address(0), spender_);
        emit GuardianSet(address(0), guardian_);
    }

    // -------------------------------------------------------------------------
    // Spending
    // -------------------------------------------------------------------------

    /// @inheritdoc IAttestCoinTreasuryVault
    /// @dev Uses `SafeERC20.forceApprove`, which is also safe against tokens that reject a
    ///      non-zero-to-non-zero allowance change.
    function setAllowance(uint256 amount) external override onlyOwner {
        token.forceApprove(spender, amount);
        emit AllowanceSet(amount);
    }

    /// @inheritdoc IAttestCoinTreasuryVault
    function pauseRewardRedemptions() external override {
        if (msg.sender != guardian && msg.sender != owner()) revert NotGuardianOrOwner();
        token.forceApprove(spender, 0);
        emit RewardRedemptionsPaused(msg.sender);
    }

    // -------------------------------------------------------------------------
    // Funds
    // -------------------------------------------------------------------------

    /// @inheritdoc IAttestCoinTreasuryVault
    /// @dev Funding needs no counterpart: transfer tokens to this address and they are usable.
    function withdraw(address to, uint256 amount) external override onlyOwner {
        if (to == address(0)) revert ZeroAddress();
        if (amount == 0) revert ZeroAmount();
        token.safeTransfer(to, amount);
        emit Withdrawn(to, amount);
    }

    // -------------------------------------------------------------------------
    // Configuration
    // -------------------------------------------------------------------------

    /// @inheritdoc IAttestCoinTreasuryVault
    /// @dev Revokes the outgoing spender's allowance first. Without that, a replaced precompile
    ///      address would retain a live claim on the vault's funds.
    function setSpender(address newSpender) external override onlyOwner {
        if (newSpender == address(0)) revert ZeroAddress();
        address previous = spender;
        if (previous != newSpender) token.forceApprove(previous, 0);
        spender = newSpender;
        emit SpenderSet(previous, newSpender);
    }

    /// @inheritdoc IAttestCoinTreasuryVault
    function setGuardian(address newGuardian) external override onlyOwner {
        address previous = guardian;
        guardian = newGuardian;
        emit GuardianSet(previous, newGuardian);
    }

    // -------------------------------------------------------------------------
    // Views
    // -------------------------------------------------------------------------

    /// @inheritdoc IAttestCoinTreasuryVault
    function availableToSpend() external view override returns (uint256) {
        uint256 balance = token.balanceOf(address(this));
        uint256 allowance = token.allowance(address(this), spender);
        return balance < allowance ? balance : allowance;
    }

    // -------------------------------------------------------------------------
    // Ownership
    //
    // `IAttestCoinTreasuryVault` restates the ownership surface so callers can reach it through
    // that interface. Solidity then sees two base definitions per function, so each needs an
    // explicit override. These forward to OpenZeppelin unchanged — including its access control,
    // which lives on the `super` implementations — and add no logic of their own.
    // -------------------------------------------------------------------------

    /// @inheritdoc IAttestCoinTreasuryVault
    function owner() public view override(IAttestCoinTreasuryVault, Ownable) returns (address) {
        return super.owner();
    }

    /// @inheritdoc IAttestCoinTreasuryVault
    function pendingOwner()
        public
        view
        override(IAttestCoinTreasuryVault, Ownable2Step)
        returns (address)
    {
        return super.pendingOwner();
    }

    /// @inheritdoc IAttestCoinTreasuryVault
    function transferOwnership(address newOwner)
        public
        override(IAttestCoinTreasuryVault, Ownable2Step)
    {
        super.transferOwnership(newOwner);
    }

    /// @inheritdoc IAttestCoinTreasuryVault
    function acceptOwnership() public override(IAttestCoinTreasuryVault, Ownable2Step) {
        super.acceptOwnership();
    }
}
