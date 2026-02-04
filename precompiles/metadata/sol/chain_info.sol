// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The precompiled address of the ChainInformationPrecompile contract.
address constant CHAIN_INFO_ADDRESS = 0x0000000000000000000000000000000000000fD3;

/// @dev Instance of the ChainInfoContract interface at the precompiled address.
ChainInfoContract constant CHAIN_INFO_CONTRACT_ADRRESS = ChainInfoContract(CHAIN_INFO_ADDRESS);

/**
 * @dev Chain information structure
 */
struct ChainInfo {
    uint64 chainKey;
    uint64 chainId;
    bytes chainName;
    uint8 chainEncoding;
}

/**
 * @dev Chain information result structure
 */
struct ChainInfoResult {
    ChainInfo info;
    bool exists;
}

/**
 * @dev Height result structure
 */
struct HeightResult {
    uint64 height;
    bool exists;
}

/**
 * @dev Height with hash result structure
 */
struct HeightHashResult {
    uint64 height;
    bytes32 hash; // The agreed-upon digest/hash
    bool exists;
}

/**
 * @dev Closest height result structure NOT SURE IF WE ARE GOING TO NEED THIS
 */
struct ClosestHeightResult {
    uint64 height;
    bytes32 hash; // The agreed-upon digest/hash
    bool found;
    bool isAttestation; // true for attestation, false for checkpoint
}

/**
 * @dev Bounds check result structure
 */
struct BoundsCheckResult {
    uint64 parentHeight;
    bytes32 parentHash;
    bool parentIsAttestation;
    uint64 childHeight;
    bytes32 childHash;
    bool childIsAttestation;
    bool isAttested;
}

/**
 * @title ChainInfoContract
 * @dev Interface for the Chain Information Precompile
 * @notice This precompile provides functionality to query chain information and attestation/checkpoint data
 * @notice Precompile Address: 0x0000000000000000000000000000000000000fD3
 */
interface ChainInfoContract {
    /**
     * @dev Get list of all supported chains
     * @return chains Array of all supported chains
     */
    function get_supported_chains() external view returns (ChainInfo[] memory chains);

    /**
     * @dev Get specific chain information by chain ID
     * @param chainKey The chain Key to look up
     * @return result Chain information if found
     */
    function get_chain_by_key(uint64 chainKey) external view returns (ChainInfoResult memory result);

    /**
     * @dev Get attestation genesis height for a chain
     * @param chainKey The chain key to query
     * @return genesisHeight The genesis height for attestations
     */
    function get_attestation_genesis_height(uint64 chainKey) external view returns (uint64 genesisHeight);

    /**
     * @dev Get latest attestation height and hash for a chain
     * @param chainKey The chain key to query
     * @return result Latest attestation height and agreed-upon hash
     */
    function get_latest_attestation_height_and_hash(uint64 chainKey)
        external
        view
        returns (HeightHashResult memory result);

    /**
     * @dev Get latest checkpoint height and hash for a chain
     * @param chainKey The chain key to query
     * @return result Latest checkpoint height and agreed-upon hash
     */
    function get_latest_checkpoint_height_and_hash(uint64 chainKey)
        external
        view
        returns (HeightHashResult memory result);

    /**
     * @dev Find highest attested block BEFORE target height
     * @param chainKey The chain key to query
     * @param targetHeight The target height to find parent for
     * @return result Highest attested height < targetHeight
     */
    function find_highest_attested_before(uint64 chainKey, uint64 targetHeight)
        external
        view
        returns (HeightHashResult memory result);

    /**
     * @dev Find lowest attested block AFTER target height
     * @param chainKey The chain key to query
     * @param targetHeight The target height to find child for
     * @return result Lowest attested height > targetHeight
     */
    function find_lowest_attested_after(uint64 chainKey, uint64 targetHeight)
        external
        view
        returns (HeightHashResult memory result);

    /**
     * @dev Check if a specific height is attested (has continuity proof)
     * @param chainKey The chain key to query
     * @param targetHeight The height to check
     * @return isAttested Whether the height can be proven via continuity
     */
    function is_height_attested(uint64 chainKey, uint64 targetHeight) external view returns (bool isAttested);

    /**
     * @dev Get attestation bounds for a target height
     * @notice Returns the parent (highest attested before) and child (lowest attested after) for a target height
     * @param chainKey The chain key to query
     * @param targetHeight The target height to find bounds for
     * @return result Attestation bounds result
     */
    function get_attestation_bounds(uint64 chainKey, uint64 targetHeight)
        external
        view
        returns (BoundsCheckResult memory result);

    /**
     * @dev Get attestation height by its digest
     * @param chainKey The chain key to query
     * @param digest The attestation digest to look up
     * @return result Attestation height result
     */
    function get_attestation_height_for_digest(uint64 chainKey, bytes32 digest) external view returns (HeightResult memory);

    /**
     * @dev Get checkpoint by its digest/hash
     * @param chainKey The chain key to query
     * @param height The checkpoint height to look up
     * @return result Checkpoint height result
     */
    function get_checkpoint_for_height(uint64 chainKey, uint64 height) external view returns (HeightHashResult memory);
}
