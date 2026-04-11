// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0 <0.9.0;

/**
 * @title BridgeMessageTypeRegistry
 * @notice Abstract registry contract that manages which bridge message types are
 *         permitted for processing via bridgeTo.
 * @dev Supported message type values:
 *  - 0: TOKEN_TRANSFER       Cross-chain token transfer only (mint on destination / burn on source).
 *  - 1: PAYLOAD_EXECUTION    Execute a payload on the destination contract, no token transfer.
 *  - 2: TOKEN_TRANSFER_WITH_PAYLOAD  Cross-chain token transfer combined with a destination payload execution.
 */
abstract contract BridgeMessageTypeRegistry {
    error MsgTypeAlreadyAllowed(uint8 msgType);
    error MsgTypeNotAllowed(uint8 msgType);

  
    mapping(uint8 => bool) private _allowedMsgTypes;

    event MsgTypeAllowed(uint8 indexed msgType);
    event MsgTypeDisallowed(uint8 indexed msgType);
   

    function isAllowed(uint8 msgType) external view returns (bool) {
        return _allowedMsgTypes[msgType];
    }

    function allowMsgType(uint8 msgType) external virtual {
        if (_allowedMsgTypes[msgType]) {
            revert MsgTypeAlreadyAllowed(msgType);
        }
        _allowedMsgTypes[msgType] = true;
        emit MsgTypeAllowed(msgType);
    }

    function disallowMsgType(uint8 msgType) external virtual {
        if (!_allowedMsgTypes[msgType]) {
            revert MsgTypeNotAllowed(msgType);
        }
        _allowedMsgTypes[msgType] = false;
        emit MsgTypeDisallowed(msgType);
    }
}
