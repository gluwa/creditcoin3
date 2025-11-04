# Query-CLI Refactoring Summary

## Overview

The query-cli has been refactored to improve code organization, maintainability, and separation of concerns. The monolithic `main.rs` file has been split into focused modules, each handling a specific aspect of the query execution flow.

## New Module Structure

### 1. `config.rs` - Configuration Management
- **Purpose**: Centralized configuration types and network settings
- **Key Types**:
  - `Network`: Enum for different network types (Sepolia, Ethereum, Local, Custom)
  - `DataSelection`: Enum for different data selection strategies
  - `QueryConfig`: Configuration for individual queries
  - `CreditcoinConfig`: Creditcoin3 chain configuration
  - `AppConfig`: Top-level application configuration
- **Responsibilities**:
  - Network identification and RPC URL management
  - Converting configuration to `Query` objects
  - Layout segment generation based on data selection

### 2. `prompt.rs` - User Interaction
- **Purpose**: Handle all user prompts and input collection
- **Key Types**:
  - `PromptArgs`: Arguments that can be provided via CLI or interactively
  - `PromptOutput`: Collected user input
  - `SelectedData`: Enum for data selection choices
- **Responsibilities**:
  - Network selection (Sepolia, Ethereum, Local, Custom)
  - Block height and transaction hash collection
  - Data selection (All, Range, ERC20 Transfer, Native Token Transfer)
  - Input validation and formatting

### 3. `merkle.rs` - Merkle Proof Generation
- **Purpose**: Generate and prepare Merkle proofs for transaction verification
- **Key Functions**:
  - `generate_merkle_proof()`: Creates Merkle proof with proper sibling formatting
  - `get_transaction_data()`: Extracts transaction data from blocks
  - `display_block_info()`: Shows block structure for debugging
- **Responsibilities**:
  - Building Merkle trees from blocks using Starknet Pedersen MMR
  - Generating proofs with placeholder values at offset positions
  - Transaction data extraction

### 4. `continuity.rs` - Continuity Proof Generation
- **Purpose**: Fetch attestations and build continuity proofs from Creditcoin3 chain
- **Key Functions**:
  - `fetch_continuity_proof()`: Main entry point for continuity proof generation
  - `find_continuity_bounds()`: Locates attestations/checkpoints around query height
  - `build_continuity_fragment()`: Constructs continuity chain
  - `compute_block_digest()`: Computes digest for blocks in continuity chain
- **Responsibilities**:
  - Connecting to Creditcoin3 chain to fetch attestations
  - Finding lower and upper bounds for continuity proof
  - Building continuity blocks with proper digest computation
  - Handling cases where attestations are not yet available

### 5. `verification.rs` - Query Verification
- **Purpose**: Handle query verification against the native query verifier precompile
- **Key Types**:
  - `VerificationConfig`: Configuration for verification
  - `VerificationResult`: Result of verification with success status and segments
- **Key Functions**:
  - `verify_query()`: Calls the native query verifier precompile
  - `display_results()`: Shows verification results in user-friendly format
- **Responsibilities**:
  - Interfacing with the native query verifier contract
  - Gas estimation (optional)
  - Result formatting and display
  - Error handling for missing continuity data

### 6. `native_query.rs` - Native Query Execution Flow
- **Purpose**: Orchestrate the complete native query execution flow
- **Key Functions**:
  - `execute_native_query()`: Main execution flow
  - `fetch_block_data()`: Fetches block from source chain
  - `find_transaction_index()`: Locates transaction in block
- **Sub-modules**:
  - `submission`: Handles interactive native query submission
- **Responsibilities**:
  - Coordinating all steps of native query verification
  - Block data fetching
  - Transaction index finding
  - Calling merkle, continuity, and verification modules
  - Progress logging and debugging output

### 7. `query_builder.rs` - Query Construction Helpers
- **Purpose**: Build layout segments for specific query types
- **Key Functions**:
  - `get_erc20_transfer_segments()`: Extracts ERC20 transfer event data locations
  - `get_native_token_transfer_segments()`: Extracts native token transfer data locations
- **Key Types**:
  - `BlockscoutAbiProvider`: Fetches ABIs from Blockscout
  - `PocAbiProvider`: Local ABI provider for testing
- **Responsibilities**:
  - ABI retrieval for contract interactions
  - Smart contract event parsing
  - Layout segment generation for common query patterns

### 8. `main.rs` - Entry Point and CLI
- **Purpose**: CLI argument parsing and command routing
- **Key Responsibilities**:
  - Parsing command-line arguments with clap
  - Routing to appropriate subcommands (Prover vs Native)
  - Setting up logging
  - Top-level error handling

## Command Structure

### Verify Command (Native Query Verification)
```bash
query-cli --cc3-rpc-url <URL> --cc3-evm-private-key <KEY> verify \
  [--eth-rpc-url <URL>] \
  [--block-height <HEIGHT>] \
  [--txn-hash <HASH>] \
  [--data-choice <CHOICE>]
```

The verify command uses the native query verifier precompile at address 0x0FD2 for direct, efficient verification without requiring external proof generation.

## Benefits of the Refactor

1. **Separation of Concerns**: Each module has a single, well-defined responsibility
2. **Testability**: Smaller, focused functions are easier to unit test
3. **Maintainability**: Changes to one aspect (e.g., Merkle proof generation) don't affect others
4. **Reusability**: Modules can be used independently or in different combinations
5. **Readability**: Code is organized logically, making it easier to understand the flow
6. **Extensibility**: New features can be added without modifying existing modules

## Migration Notes

### For Developers

- **Old approach**: Everything in `main.rs` with large functions
- **New approach**: Import specific modules and use their public APIs
- **Example**:
  ```rust
  // Old
  let merkle_proof = /* inline merkle proof generation */

  // New
  use crate::merkle;
  let merkle_proof = merkle::generate_merkle_proof(&block, tx_index)?;
  ```

### Breaking Changes

- `Network` enum is now in `main.rs` (used by multiple modules)
- `PromptArgs` is now public (was `pub(crate)`)
- Network chain IDs updated to correct values (Sepolia: 11155111 instead of 3)

## Future Improvements

1. **Configuration Files**: Support for TOML/YAML configuration files
2. **Better Error Types**: Custom error types for each module
3. **Async Optimization**: Better async/await patterns and concurrency
4. **Testing**: Comprehensive unit and integration tests for each module
5. **Documentation**: More inline documentation and examples
6. **Native Query Integration**: Complete integration of refactored native query flow

## Testing

To test the refactored code:

```bash
# Build the project
cargo build --package query-cli

# Run with native verification
cargo run --package query-cli -- \
  --cc3-rpc-url ws://localhost:9944 \
  --cc3-evm-private-key <your-key> \
  native \
  --eth-rpc-url ws://localhost:8545 \
  --block-height 12345 \
  --txn-hash 0xabc...
```

## Dependencies Added

- `hex`: For hexadecimal encoding/decoding (already in workspace)

## Files Modified

- `query-cli/Cargo.toml`: Added `hex` dependency
- `query-cli/src/main.rs`: Refactored to use new modules, fixed imports
- `query-cli/src/config.rs`: Created (configuration types)
- `query-cli/src/continuity.rs`: Created (continuity proof generation)
- `query-cli/src/merkle.rs`: Created (Merkle proof generation)
- `query-cli/src/native_query.rs`: Created (native query execution)
- `query-cli/src/verification.rs`: Created (verification logic)
- `query-cli/src/prompt.rs`: Already existed, minor updates
- `query-cli/src/query_builder.rs`: Already existed, no changes needed

## Status

✅ Refactoring complete
✅ All files compile without errors
✅ Module structure established
✅ Dependencies resolved
✅ Public APIs defined

The refactor is complete and ready for further development and testing!
