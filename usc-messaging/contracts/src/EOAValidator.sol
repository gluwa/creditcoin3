// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./IVoteValidator.sol";

/// @notice EOA (ECDSA / `ecrecover`) vote validator — the production replacement for
/// `DummyVoteValidator`. Validates that a threshold of authorized attestors signed the message
/// hash. Suitable for a low attestor count; can be swapped for a BLS/TSS validator later by
/// pointing the Inbox at a different `IVoteValidator` (research §12).
///
/// Votes are `abi.encode(bytes[] signatures)`, each a 65-byte `(r, s, v)` ECDSA signature over the
/// raw `messageHash` (no EIP-191 / personal_sign prefix) — byte-identical to what the Rust attestor
/// produces and the relayer assembles.
contract EOAValidator is IVoteValidator {
    /// Administrative owner who can override the attestor set / threshold (emergency mechanism).
    address public admin;

    /// Minimum required signatures, regardless of the threshold fraction (security floor).
    uint256 public minAttestorCount;

    /// Threshold fraction + addition, e.g. 2/3 + 1.
    uint256 public thresholdNumerator;
    uint256 public thresholdDenominator;
    uint256 public thresholdAddition;

    /// O(1) membership for `ecrecover` checks.
    mapping(address => bool) public isAttestor;
    /// Iterable attestor set (kept in sync with `isAttestor`). Exposed via `attestors()`.
    address[] private _attestors;

    /// Monotonic nonce bound into every `submitAttestorSetUpdate` signed payload. Incremented on
    /// each successful update so a previously-signed (and applied) set change cannot be replayed to
    /// roll the attestor set back. Signers must sign against the *current* value.
    uint256 public attestorSetUpdateNonce;

    event AttestorSetUpdated(address[] newAttestors);
    event ThresholdUpdated(uint256 numerator, uint256 denominator, uint256 addition);
    event MinAttestorCountUpdated(uint256 newMin);

    /// secp256k1 group order ÷ 2 — the EIP-2 upper bound on a non-malleable `s`.
    uint256 internal constant SECP256K1_HALF_N =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    error NotAdmin();
    error InvalidArgument(string reason);
    error InvalidSignatureLength();
    error InvalidAttestor();
    error DoubleSigning();
    error ThresholdNotMet(uint256 got, uint256 required);
    error MalleableSignature();

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    constructor(
        address _admin,
        address[] memory _initialAttestors,
        uint256 _minAttestorCount,
        uint256 _thresholdNumerator,
        uint256 _thresholdDenominator,
        uint256 _thresholdAddition
    ) {
        if (_admin == address(0)) revert InvalidArgument("admin");
        if (_thresholdDenominator == 0) revert InvalidArgument("denominator");
        if (_initialAttestors.length < _minAttestorCount) revert InvalidArgument("below minimum");

        admin = _admin;
        minAttestorCount = _minAttestorCount;
        thresholdNumerator = _thresholdNumerator;
        thresholdDenominator = _thresholdDenominator;
        thresholdAddition = _thresholdAddition;

        for (uint256 i = 0; i < _initialAttestors.length; i++) {
            _addAttestor(_initialAttestors[i]);
        }
    }

    // ----------------------------------- IVoteValidator ----------------------------------------- //

    /// @inheritdoc IVoteValidator
    /// @dev Decodes `votes` as `bytes[]`, `ecrecover`s each 65-byte signature against `messageHash`,
    /// requires each signer to be an attestor, rejects a signer appearing twice in this call, and
    /// requires at least `threshold()` unique signers.
    function validateVotes(bytes32 messageHash, bytes calldata votes) external view override {
        bytes[] memory signatures = abi.decode(votes, (bytes[]));

        address[] memory seen = new address[](signatures.length);
        uint256 unique = 0;

        for (uint256 i = 0; i < signatures.length; i++) {
            // Direct hash signing — no EIP-191 prefix (matches the attestor signer).
            address signer = _recoverChecked(messageHash, signatures[i]);
            if (!isAttestor[signer]) revert InvalidAttestor();

            for (uint256 j = 0; j < unique; j++) {
                if (seen[j] == signer) revert DoubleSigning();
            }
            seen[unique] = signer;
            unique++;
        }

        uint256 required = calculateRequiredVotes(_attestors.length);
        if (unique < required) revert ThresholdNotMet(unique, required);
    }

    /// @inheritdoc IVoteValidator
    function attestors() external view override returns (address[] memory) {
        return _attestors;
    }

    /// @inheritdoc IVoteValidator
    function threshold() external view override returns (uint256) {
        return calculateRequiredVotes(_attestors.length);
    }

    /// @notice Required unique signatures for `totalAttestors`: `max(floor(N*num/den)+add, minimum)`.
    function calculateRequiredVotes(uint256 totalAttestors) public view returns (uint256) {
        uint256 t = (totalAttestors * thresholdNumerator / thresholdDenominator) + thresholdAddition;
        return t > minAttestorCount ? t : minAttestorCount;
    }

    // -------------------------------------- Admin overrides ------------------------------------- //

    function addAttestor(address attestor) external onlyAdmin {
        _addAttestor(attestor);
        emit AttestorSetUpdated(_attestors);
    }

    function removeAttestor(address attestor) external onlyAdmin {
        if (!isAttestor[attestor]) revert InvalidAttestor();
        if (_attestors.length - 1 < minAttestorCount) revert InvalidArgument("below minimum");
        _removeAttestor(attestor);
        emit AttestorSetUpdated(_attestors);
    }

    /// @notice Replace the entire attestor set.
    function updateAttestorSet(address[] calldata newAttestors) external onlyAdmin {
        if (newAttestors.length < minAttestorCount) revert InvalidArgument("below minimum");
        _replaceAttestorSet(newAttestors);
        emit AttestorSetUpdated(newAttestors);
    }

    function updateThreshold(uint256 _numerator, uint256 _denominator, uint256 _addition)
        external
        onlyAdmin
    {
        if (_denominator == 0) revert InvalidArgument("denominator");
        thresholdNumerator = _numerator;
        thresholdDenominator = _denominator;
        thresholdAddition = _addition;
        emit ThresholdUpdated(_numerator, _denominator, _addition);
    }

    function updateMinAttestorCount(uint256 _minAttestorCount) external onlyAdmin {
        if (_attestors.length < _minAttestorCount) {
            revert InvalidArgument("current set below minimum");
        }
        minAttestorCount = _minAttestorCount;
        emit MinAttestorCountUpdated(_minAttestorCount);
    }

    // --------------------------------- Attestor-voted set update -------------------------------- //

    /// @notice Replace the attestor set with one signed by a threshold of the *current* attestors
    /// (more decentralized than `updateAttestorSet`; anyone may submit, signatures must clear the
    /// current threshold). Sign `keccak256(abi.encode(newAttestors, block.chainid, nonce))` directly,
    /// where `nonce` is the current `attestorSetUpdateNonce`. The nonce is chain-id bound (no
    /// cross-chain replay) and monotonic (no rollback replay: once applied, the signed payload is
    /// spent).
    function submitAttestorSetUpdate(address[] calldata newAttestors, bytes calldata signatures)
        external
    {
        if (newAttestors.length < minAttestorCount) revert InvalidArgument("below minimum");

        bytes32 updateHash =
            keccak256(abi.encode(newAttestors, block.chainid, attestorSetUpdateNonce));
        bytes[] memory sigs = abi.decode(signatures, (bytes[]));

        address[] memory seen = new address[](sigs.length);
        uint256 unique = 0;
        for (uint256 i = 0; i < sigs.length; i++) {
            address signer = _recoverChecked(updateHash, sigs[i]);
            if (!isAttestor[signer]) revert InvalidAttestor();
            for (uint256 j = 0; j < unique; j++) {
                if (seen[j] == signer) revert DoubleSigning();
            }
            seen[unique] = signer;
            unique++;
        }

        uint256 required = calculateRequiredVotes(_attestors.length);
        if (unique < required) revert ThresholdNotMet(unique, required);

        // Spend the nonce before applying so a replay of this exact payload reverts on InvalidAttestor
        // (recovers against a stale hash) on any later call.
        unchecked {
            attestorSetUpdateNonce++;
        }
        _replaceAttestorSet(newAttestors);
        emit AttestorSetUpdated(newAttestors);
    }

    // ------------------------------------- View helpers ----------------------------------------- //

    function getAttestorCount() external view returns (uint256) {
        return _attestors.length;
    }

    // -------------------------------------- Internals ------------------------------------------- //

    /// @dev Recover a signer from a 65-byte `(r, s, v)` signature with EIP-2 hardening: reject a
    /// high-`s` (malleable) value and any `v` outside {27, 28}, plus a zero-address recovery. Shared
    /// by both signature-checking loops so they can't drift. Not a threshold bypass even without it
    /// (both this contract and the relayer dedup by the *recovered* address, so a malleated copy
    /// recovers to the same signer and can't inflate the unique count) — this is defense-in-depth so
    /// the guarantee survives any future refactor of the dedup logic.
    function _recoverChecked(bytes32 hash, bytes memory sig) internal pure returns (address) {
        if (sig.length != 65) revert InvalidSignatureLength();
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }
        if (uint256(s) > SECP256K1_HALF_N) revert MalleableSignature();
        if (v != 27 && v != 28) revert MalleableSignature();
        address signer = ecrecover(hash, v, r, s);
        if (signer == address(0)) revert InvalidAttestor();
        return signer;
    }

    function _addAttestor(address attestor) internal {
        if (attestor == address(0)) revert InvalidArgument("attestor");
        if (isAttestor[attestor]) revert InvalidArgument("duplicate attestor");
        isAttestor[attestor] = true;
        _attestors.push(attestor);
    }

    function _removeAttestor(address attestor) internal {
        isAttestor[attestor] = false;
        for (uint256 i = 0; i < _attestors.length; i++) {
            if (_attestors[i] == attestor) {
                _attestors[i] = _attestors[_attestors.length - 1];
                _attestors.pop();
                break;
            }
        }
    }

    function _replaceAttestorSet(address[] memory newAttestors) internal {
        for (uint256 i = 0; i < _attestors.length; i++) {
            isAttestor[_attestors[i]] = false;
        }
        delete _attestors;
        for (uint256 i = 0; i < newAttestors.length; i++) {
            _addAttestor(newAttestors[i]);
        }
    }
}
