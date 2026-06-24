// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../src/EOAValidator.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function addr(uint256 privateKey) external returns (address);
    function sign(uint256 privateKey, bytes32 digest)
        external
        returns (uint8 v, bytes32 r, bytes32 s);
    function expectRevert() external;
}

contract EOAValidatorTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    EOAValidator validator;
    uint256 constant K1 = 0xA11CE;
    uint256 constant K2 = 0xB0B;
    uint256 constant K3 = 0xC4A12;
    uint256 constant K_OUTSIDER = 0xDEAD;

    bytes32 constant MSG_HASH = keccak256("some message hash");

    function setUp() public {
        address[] memory init = new address[](3);
        init[0] = vm.addr(K1);
        init[1] = vm.addr(K2);
        init[2] = vm.addr(K3);
        // admin = this test, minAttestorCount = 1, threshold = 2/3 + 1.
        validator = new EOAValidator(address(this), init, 1, 2, 3, 1);
    }

    /// 65-byte `(r, s, v)` ECDSA signature over `h` — matches what the Rust attestor produces.
    function _sig(uint256 key, bytes32 h) internal returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, h);
        return abi.encodePacked(r, s, v);
    }

    function _votes(bytes[] memory sigs) internal pure returns (bytes memory) {
        return abi.encode(sigs);
    }

    // --- views match the Rust IVoteValidator binding -------------------------------------------- //

    function test_views() public {
        require(validator.attestors().length == 3, "attestors() length");
        // (3 * 2 / 3) + 1 = 3, and 3 > min(1) -> threshold 3.
        require(validator.threshold() == 3, "threshold");
    }

    // --- happy path ----------------------------------------------------------------------------- //

    function test_quorum_of_three_passes() public {
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(K1, MSG_HASH);
        sigs[1] = _sig(K2, MSG_HASH);
        sigs[2] = _sig(K3, MSG_HASH);
        validator.validateVotes(MSG_HASH, _votes(sigs)); // must not revert
    }

    // --- failure modes -------------------------------------------------------------------------- //

    function test_below_threshold_reverts() public {
        bytes[] memory sigs = new bytes[](2); // need 3
        sigs[0] = _sig(K1, MSG_HASH);
        sigs[1] = _sig(K2, MSG_HASH);
        vm.expectRevert();
        validator.validateVotes(MSG_HASH, _votes(sigs));
    }

    function test_non_attestor_reverts() public {
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(K1, MSG_HASH);
        sigs[1] = _sig(K2, MSG_HASH);
        sigs[2] = _sig(K_OUTSIDER, MSG_HASH); // not in the set
        vm.expectRevert();
        validator.validateVotes(MSG_HASH, _votes(sigs));
    }

    function test_duplicate_signer_reverts() public {
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(K1, MSG_HASH);
        sigs[1] = _sig(K1, MSG_HASH); // K1 twice
        sigs[2] = _sig(K2, MSG_HASH);
        vm.expectRevert();
        validator.validateVotes(MSG_HASH, _votes(sigs));
    }

    function test_wrong_hash_does_not_count() public {
        // Signatures over a different hash recover to non-attestor addresses -> reverts.
        bytes32 other = keccak256("different");
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(K1, other);
        sigs[1] = _sig(K2, other);
        sigs[2] = _sig(K3, other);
        vm.expectRevert();
        validator.validateVotes(MSG_HASH, _votes(sigs));
    }

    function test_bad_signature_length_reverts() public {
        bytes[] memory sigs = new bytes[](1);
        sigs[0] = hex"1234"; // not 65 bytes
        vm.expectRevert();
        validator.validateVotes(MSG_HASH, _votes(sigs));
    }
}
