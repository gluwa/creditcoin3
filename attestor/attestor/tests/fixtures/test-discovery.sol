// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.26;
contract TestDiscovery {
    mapping(address => bool) public active;
    function setActive(address outbox, bool yes) external { active[outbox] = yes; }
    function isActiveOutbox(uint32, address outbox) external view returns (bool) { return active[outbox]; }
    function get_outbox_discovery_address(uint64) external view returns (address, bool) { return (address(this), true); }
}
