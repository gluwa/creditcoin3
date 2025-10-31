# Proof Generation Flow and Block Data Sourcing

**Last Updated:** January 2025

## Executive Summary

This document explains how proof generation works in Creditcoin3, specifically focusing on:
1. How block data is obtained for proof generation
2. The flow from query submission to proof verification
3. How a native precompile approach would differ from the current STARK approach
4. Architecture options for automatic query generation from smart contract events

---

## Table of Contents
1. [Current Architecture (STARK-based)](#current-architecture-stark-based)
2. [Block Data Sourcing](#block-data-sourcing)
3. [Complete Proof Generation Flow](#complete-proof-generation-flow)
4. [Native Precompile Architecture](#native-precompile-architecture)
5. [Automatic Query Generation from Events](#automatic-query-generation-from-events)
6. [Design Patterns and Solutions](#design-patterns-and-solutions)

---

## Current Architecture (STARK-based)

### System Components

```
┌─────────────────────────────────────────────────────────────┐
│                                                               │
│  Creditcoin3 Chain (EVM + Substrate)                        │
│  ┌─────────────────┐         ┌──────────────────┐          │
│  │ Prover Pallet   │◄───────►│ Verifier         │          │
│  │ (Query Storage) │         │ Precompile       │          │
│  └─────────────────┘         │ (0x0Be9)         │          │
│                               └──────────────────┘          │
│                                                               │
└───────────────────┬───────────────────────────────────────────┘
                    │
                    │ (3) Fetch Queries
                    ▼
        ┌───────────────────────┐
        │   Prover Service      │◄──────┐
        │   (Off-chain)         │       │
        └───────┬───────────────┘       │
                │                        │
                │ (4) Fetch Block Data   │ (6) Submit Proof
                │                        │
                ▼                        │
   ┌────────────────────────┐          │
   │  Source Chain          │          │
   │  (Ethereum/Polygon)    │          │
   │  - RPC Endpoint        │          │
   │  - Block Data          │          │
   │  - Tx Receipts         │          │
   └────────────────────────┘          │
                │                        │
                │ (5) Generate Proof     │
                ▼                        │
        ┌───────────────────┐           │
        │  Cairo Program     │           │
        │  Stone Prover      │───────────┘
        └───────────────────┘
```

### Key Characteristics

- **Off-chain Processing**: Prover service runs as a separate process
- **Direct RPC Access**: Fetches block data directly from source chain
- **STARK Proof**: Generates cryptographic proofs using Cairo + Stone
- **On-chain Verification**: Verifies proofs using native precompile
- **Trust Model**: Trustless - proofs are cryptographically verifiable by anyone

---

## Block Data Sourcing

### How Block Data is Obtained

The prover service fetches block data through the following mechanism:

```rust
// From: prover/src/query/mod.rs

pub async fn process(
    eth_client: &Client,              // RPC client for source chain
    query: &Query,                     // Query specifying block/tx to prove
    attestation_cache: &AttestationCacheType,
    stone_proof: bool,
    encoding: EncodingVersion,
) -> Result<Either<Proof, Vec<PathBuf>>, Error> {

    // 1. Get attestation fragment (continuity chain)
    let attestation_fragment =
        fragment::get_for_query(eth_client, query, attestation_cache, encoding).await?;

    // 2. Fetch the actual block data from source chain via RPC
    let block = eth_client.get_block(query.height, encoding).await?;

    // 3. Build proof
    let query_prover =
        proof::run_cairo_verifier(query_serializable, &attestation_fragment, block).await?;

    // 4. Generate STARK proof
    // ...
}
```

### FragmentManager: Parallel Block Fetching

```rust
// From: common/eth/src/continuity.rs

impl<'a> Manager<'a> {
    pub async fn create(
        &self,
        prev_digest: H256,
        encoding: EncodingVersion,
    ) -> Result<AttestationFragment, Error> {

        // Fetch all blocks in parallel from source chain
        let blocks = futures::future::join_all(
            (self.start_block..=self.end_block)
                .map(|i| self.eth_client.get_block(i, encoding))
        ).await;

        // Compute merkle roots in parallel
        let blocks_with_roots = stream::iter(collected_blocks)
            .map(|block| {
                tokio::task::spawn_blocking(move || {
                    let root = crate::starknet_pedersen_mmr(&block);
                    (block, root)
                })
            })
            .buffered(10)
            .collect::<Vec<_>>()
            .await;

        // Build attestation fragment
        for block_with_root in blocks_with_roots {
            let (block, merkle_root) = block_with_root?;
            fragment.try_append_block(FragmentBlock::new(
                block.number(),
                merkle_root.root().0
            ))?;
        }

        Ok(fragment)
    }
}
```

### What Data is Fetched

For each block, the following data is retrieved:

```rust
pub struct OrderedBlock {
    pub block: Block<Transaction>,      // Full block with transactions
    pub receipts: Vec<TransactionReceipt>, // Transaction receipts
}
```

The block data includes:
- **Block header**: timestamp, parent hash, state root, etc.
- **Transactions**: Full transaction data (from, to, value, data, etc.)
- **Receipts**: Transaction execution results, logs, gas used
- **Merkle Tree**: Built from transactions + receipts for proof generation

---

## Complete Proof Generation Flow

### Step-by-Step Process

#### 1. Query Submission (On-chain)

A user or contract submits a query to the Prover Pallet:

```solidity
// Solidity contract submitting a query
function submitQuery(
    uint64 chainId,        // Source chain (e.g., Ethereum = 1)
    uint64 blockNumber,    // Block to query
    uint64 txIndex,        // Transaction index in block
    bytes[] layoutSegments // Data segments to extract
) external {
    // Calls Prover Pallet via precompile
    ProverPrecompile.submitQuery(chainId, blockNumber, txIndex, layoutSegments);
}
```

The query is stored on-chain and emits an event.

#### 2. Query Detection (Off-chain)

The prover service listens for new queries:

```rust
// From: prover/src/lib.rs

loop {
    tokio::select! {
        Some(query) = query_submission_stream.next() => {
            let query = query?;
            // New query received from subscription
            self.handle_new_query(query, queries_to_process_sender.clone()).await?;
        },
        // ...
    }
}
```

#### 3. Attestation Check

Before processing, the prover checks if necessary attestations exist:

```rust
async fn handle_new_query(
    &mut self,
    query: Query,
    query_sender: mpsc::UnboundedSender<Query>,
) -> Result<()> {
    let last_attestation_height = self.attestations_cache
        .last_synced_attestation(query.chain_id)
        .await?;

    if last_attestation_height < query.height {
        // Wait for attestations to catch up
        self.waiting_queries
            .entry(query.height)
            .or_default()
            .push(query);
    } else {
        // Ready to process immediately
        query_sender.send(query)?;
    }

    Ok(())
}
```

#### 4. Block Data Fetching

Once ready, the prover fetches block data:

```rust
async fn handle_query_to_process(
    &mut self,
    query: Query,
    light_prover_queries: &mut LightProverQueries,
) -> Result<()> {

    // This internally fetches block data via RPC
    let r = query::process(
        &self.source_chain_eth_client,  // RPC client
        &query,
        &self.attestations_cache,
        !self.is_light_prover_mode(),
        self.encoding,
    ).await?;

    // ...
}
```

#### 5. Merkle Tree Construction

```rust
// From: proof/src/query_prover.rs

pub async fn build_prover(
    query: QuerySerializable,
    fragment_blocks: FragmentContinuityBlocksSerializable,
    block: OrderedBlock,  // Block data fetched via RPC
) -> Result<QueryProver, QueryProverError> {

    // Build Merkle tree from block transactions + receipts
    let mt = eth_common::starknet_pedersen_mmr(&block);

    let subject_index = query.id().index() as usize;
    let subject_bytes = block
        .items()
        .get(subject_index)
        .map(eth_common::TxRx::to_bytes)
        .unwrap_or(out_of_bound_witness.to_bytes())
        .clone();

    // Generate merkle proof for the specific transaction
    let merkle_path = mt.generate_proof(subject_index);

    // Create QueryProver with all necessary data
    let instance = QueryProver::new(
        query_block_number,
        merkle_path,
        subject_bytes,
        query,
        digest_root,
        fragment_blocks,
        out_of_bounds_flag,
    );

    Ok(instance)
}
```

#### 6. Cairo Program Execution

```rust
pub async fn cairo_verify(&mut self, cairo_proof_mode: bool) -> Result<(), QueryProverError> {
    // Writes input JSON file with:
    // - Query details
    // - Merkle proof
    // - Continuity chain
    // - Block data

    run_cairo_verify_script(
        Self::verify_merkle_command(),
        dir,
        cairo_proof_mode
    ).await?;

    // Cairo program verifies:
    // 1. Merkle proof is valid
    // 2. Continuity chain is correct
    // 3. Extracted data matches query
}
```

#### 7. STARK Proof Generation

```rust
pub async fn stone_prove(&self, force_stone_proving: bool) -> Result<String, QueryProverError> {
    // Runs Stone prover (cpu_air_prover)
    // Takes 15 minutes on high-end CPU
    // Generates cryptographic proof of Cairo execution

    run_stone_prover_script(
        Self::stone_prover_command(),
        dir,
        force_stone_proving
    ).await
}
```

#### 8. Proof Submission (On-chain)

```rust
// From: prover/src/lib.rs

if let Either::Left(proof) = r {
    // Verify proof locally first
    let metadata = self.fetch_stark_metadata().await?;
    verifier_core::run_verifier(&proof, query.clone(), metadata)?;

    // Submit to contract
    self.contract_client
        .submit_proof_by_id(query_id, proof)
        .await?;
}
```

#### 9. On-chain Verification

```rust
// Verifier Precompile (0x0Be9) is called
// 1. Parses STARK proof
// 2. Verifies proof cryptographically
// 3. Validates continuity chain against attestations
// 4. Returns result segments to caller
```

---

## Native Precompile Architecture

### Key Difference from STARK

With native precompiles, verification happens entirely on-chain, but **block data must still be provided**:

```
┌─────────────────────────────────────────────────────────────┐
│  Smart Contract (on Creditcoin3)                            │
│                                                               │
│  function verifyTransaction(                                 │
│      Query query,                                            │
│      bytes blockData,           ◄──── WHO PROVIDES THIS?    │
│      MerkleProof proof,         ◄──── WHO PROVIDES THIS?    │
│      ContinuityChain continuity ◄──── WHO PROVIDES THIS?    │
│  ) external {                                                │
│      require(                                                │
│          NativePrecompile.verifyMerkle(query, blockData, proof),│
│          "Invalid merkle proof"                              │
│      );                                                       │
│      require(                                                │
│          NativePrecompile.verifyContinuity(continuity),      │
│          "Invalid continuity"                                │
│      );                                                       │
│      // Use verified data...                                 │
│  }                                                            │
└─────────────────────────────────────────────────────────────┘
```

### The Critical Question

**Q: Who provides the block data to the precompile?**

**A: You still need an off-chain component!**

The native precompile doesn't magically have access to source chain block data. Someone must:
1. Fetch block data from source chain via RPC
2. Build the merkle tree
3. Generate the merkle proof
4. Submit it as calldata to the contract

### Native Precompile API Design

```solidity
// Precompile interface at address 0x0BeA (example)
interface INativeQueryVerifier {

    // Verify a merkle proof for a specific transaction
    function verifyMerkleProof(
        bytes32 root,           // Merkle root (from attestation)
        bytes memory leaf,      // Transaction data
        uint64 index,           // Transaction index
        bytes32[] memory siblings // Merkle proof siblings
    ) external view returns (bool);

    // Verify continuity chain
    function verifyContinuity(
        uint64 queryBlock,
        ContinuityBlock[] memory blocks,
        bytes32 checkpoint,
        Attestation[] memory attestations
    ) external view returns (bool);

    // Combined verification
    function verifyQuery(
        Query memory query,
        bytes memory blockData,     // Full block data or just tx data
        MerkleProof memory proof,
        ContinuityChain memory continuity
    ) external view returns (bool, bytes memory);
}
```

### Rust Precompile Implementation

```rust
// Native precompile implementation (example)

fn verify_merkle_proof(
    root: Felt,
    leaf: Felt,
    index: u64,
    siblings: Vec<Felt>
) -> bool {
    // Pedersen hash computation
    let mut current = leaf;
    let mut path = index;

    for sibling in siblings {
        if path & 1 == 0 {
            current = pedersen_hash(current, sibling);
        } else {
            current = pedersen_hash(sibling, current);
        }
        path >>= 1;
    }

    current == root
}

fn verify_continuity(
    query_block: BlockInfo,
    continuity_blocks: Vec<ContinuityBlock>,
    checkpoint: Felt,
    attestations: Vec<Attestation>
) -> bool {
    // Verify attestation signatures
    // Verify block chain continuity
    // Verify checkpoint/attestation consistency
    // ~800K-1.5M gas
}
```

### Performance Comparison

| Metric | STARK | Native Precompile |
|--------|-------|-------------------|
| **Proof Generation** | 15 minutes | <100ms |
| **Gas Cost** | 2-5M gas | 1.5-2.5M gas |
| **Trust Model** | Trustless | Trusts CC3 validators |
| **Cross-chain** | ✅ Universal | ❌ CC3 only |

---

## Automatic Query Generation from Events

### The Challenge

**Scenario**: A smart contract wants to automatically verify data when an event occurs (e.g., loan repayment on Ethereum).

**Problem**: Who fetches the block data and generates the proof?

### Solution Patterns

#### Pattern 1: Oracle/Relayer Service (Recommended)

```
┌──────────────────────────────────────────────────────────┐
│  Source Chain (Ethereum)                                 │
│  ┌────────────────┐                                      │
│  │ Loan Contract  │                                      │
│  │                │                                      │
│  │ emit Repayment(                                       │
│  │   borrower,                                           │
│  │   amount,                                             │
│  │   timestamp                                           │
│  │ )              │                                      │
│  └────────┬───────┘                                      │
└───────────┼──────────────────────────────────────────────┘
            │
            │ (1) Event Emitted
            │
            ▼
  ┌──────────────────────┐
  │  Relayer Service     │
  │  (Off-chain)         │
  │                      │
  │  - Monitors events   │
  │  - Fetches block     │
  │  - Builds merkle     │
  │  - Generates proof   │
  └──────────┬───────────┘
            │
            │ (2) Submit Query + Proof
            │
            ▼
┌──────────────────────────────────────────────────────────┐
│  Creditcoin3 Chain                                       │
│  ┌────────────────────────────────────┐                 │
│  │ Credit Rating Contract             │                 │
│  │                                    │                 │
│  │ function handleRepaymentProof(    │                 │
│  │     Query query,                  │                 │
│  │     Proof proof,                  │                 │
│  │     ContinuityChain continuity    │                 │
│  │ ) external {                      │                 │
│  │     require(                      │                 │
│  │         msg.sender == trustedRelayer,                │
│  │         "Only relayer"            │                 │
│  │     );                             │                 │
│  │                                    │                 │
│  │     require(                      │                 │
│  │         NativePrecompile.verifyQuery(               │
│  │             query, proof, continuity               │
│  │         ),                         │                 │
│  │         "Invalid proof"            │                 │
│  │     );                             │                 │
│  │                                    │                 │
│  │     // Update credit score         │                 │
│  │     updateCreditScore(query.data); │                 │
│  │ }                                  │                 │
│  └────────────────────────────────────┘                 │
└──────────────────────────────────────────────────────────┘
```

**Implementation:**

```javascript
// Relayer service (Node.js)
const ethers = require('ethers');

// Monitor Ethereum for loan repayment events
const provider = new ethers.providers.JsonRpcProvider(ETH_RPC_URL);
const loanContract = new ethers.Contract(LOAN_ADDRESS, LOAN_ABI, provider);

loanContract.on('Repayment', async (borrower, amount, timestamp, event) => {
    console.log(`Repayment detected: ${borrower} repaid ${amount}`);

    // 1. Get block and transaction data
    const block = await provider.getBlock(event.blockNumber);
    const tx = await provider.getTransaction(event.transactionHash);
    const receipt = await provider.getTransactionReceipt(event.transactionHash);

    // 2. Build merkle tree
    const merkleTree = buildMerkleTree(block.transactions);
    const txIndex = block.transactions.indexOf(event.transactionHash);
    const merkleProof = merkleTree.getProof(txIndex);

    // 3. Get continuity chain from attestation network
    const continuityChain = await getContinuityChain(
        event.blockNumber,
        attestationCache
    );

    // 4. Submit to Creditcoin3
    const query = {
        chainId: 1, // Ethereum
        blockNumber: event.blockNumber,
        txIndex: txIndex,
        layoutSegments: encodeRepaymentData(receipt.logs)
    };

    const cc3Provider = new ethers.providers.JsonRpcProvider(CC3_RPC_URL);
    const cc3Wallet = new ethers.Wallet(RELAYER_PRIVATE_KEY, cc3Provider);
    const creditContract = new ethers.Contract(
        CREDIT_CONTRACT_ADDRESS,
        CREDIT_CONTRACT_ABI,
        cc3Wallet
    );

    const tx = await creditContract.handleRepaymentProof(
        query,
        merkleProof,
        continuityChain
    );

    await tx.wait();
    console.log(`Proof submitted for ${borrower}`);
});
```

**Advantages:**
- ✅ Fast: <1 second end-to-end with native precompiles
- ✅ Simple: Standard event monitoring pattern
- ✅ Flexible: Can handle complex logic off-chain
- ✅ Cost-effective: Only pay gas for successful proofs

**Disadvantages:**
- ⚠️ Centralization: Relies on trusted relayer
- ⚠️ Availability: Relayer must be online
- ⚠️ Security: Relayer key must be protected

#### Pattern 2: Decentralized Oracle Network

```
┌──────────────────────────────────────────────────────────┐
│  Multiple Relayer Nodes                                  │
│  ┌────────┐  ┌────────┐  ┌────────┐                     │
│  │ Node 1 │  │ Node 2 │  │ Node 3 │                     │
│  └───┬────┘  └───┬────┘  └───┬────┘                     │
│      │           │           │                           │
│      └───────────┼───────────┘                           │
│                  │                                        │
│                  ▼                                        │
│       ┌──────────────────────┐                           │
│       │ Consensus Mechanism  │                           │
│       │ (2-of-3 signatures)  │                           │
│       └──────────┬───────────┘                           │
└──────────────────┼──────────────────────────────────────┘
                   │
                   ▼
         ┌─────────────────────┐
         │  Creditcoin3 Chain  │
         │  Multi-sig Contract │
         └─────────────────────┘
```

**Implementation:**

```solidity
contract DecentralizedProofVerifier {
    mapping(address => bool) public approvedOracles;
    uint256 public requiredSignatures = 2;

    struct ProofSubmission {
        Query query;
        bytes proof;
        ContinuityChain continuity;
        address[] signers;
        bytes[] signatures;
    }

    function submitProofWithSignatures(
        ProofSubmission memory submission
    ) external {
        // Verify multiple oracles signed this proof
        require(
            submission.signers.length >= requiredSignatures,
            "Not enough signatures"
        );

        bytes32 proofHash = keccak256(abi.encode(
            submission.query,
            submission.proof,
            submission.continuity
        ));

        for (uint i = 0; i < submission.signers.length; i++) {
            require(approvedOracles[submission.signers[i]], "Invalid oracle");
            require(
                verifySignature(proofHash, submission.signatures[i], submission.signers[i]),
                "Invalid signature"
            );
        }

        // Verify the actual proof
        require(
            NativePrecompile.verifyQuery(
                submission.query,
                submission.proof,
                submission.continuity
            ),
            "Invalid proof"
        );

        // Process verified data
        processVerifiedData(submission.query);
    }
}
```

**Advantages:**
- ✅ Decentralized: No single point of failure
- ✅ Secure: Requires multiple honest nodes
- ✅ Resilient: Can tolerate some offline nodes

**Disadvantages:**
- ⚠️ Complexity: More complex to implement and maintain
- ⚠️ Cost: Higher gas costs for signature verification
- ⚠️ Latency: Must wait for multiple nodes

#### Pattern 3: User-Submitted Proofs (Pull Model)

```solidity
contract CreditRating {
    mapping(address => uint256) public creditScores;

    // Users submit their own proofs
    function submitRepaymentProof(
        Query memory query,
        bytes memory proof,
        ContinuityChain memory continuity
    ) external {
        // Verify the proof
        (bool valid, bytes memory resultSegments) =
            NativePrecompile.verifyQuery(query, proof, continuity);

        require(valid, "Invalid proof");

        // Decode result segments to verify it's about msg.sender
        (address borrower, uint256 amount) = abi.decode(
            resultSegments,
            (address, uint256)
        );

        require(borrower == msg.sender, "Not your repayment");

        // Update credit score
        creditScores[msg.sender] += calculateScoreIncrease(amount);
    }
}
```

**Advantages:**
- ✅ Fully decentralized: No trusted party needed
- ✅ Privacy: Users control their own data
- ✅ Permissionless: Anyone can submit proofs

**Disadvantages:**
- ⚠️ User burden: Users must run/access proof generation
- ⚠️ UX: More complex user experience
- ⚠️ Gas: Users pay gas costs

#### Pattern 4: Hybrid (Query Registration + Prover Network)

**Current Creditcoin3 model** - works with both STARK and native precompiles:

```solidity
contract LoanContract {
    IProverPallet public prover;

    // Contract registers a query on-chain
    function requestRepaymentVerification(
        uint64 chainId,
        uint64 blockNumber,
        uint64 txIndex
    ) external {
        bytes[] memory layoutSegments = new bytes[](2);
        layoutSegments[0] = abi.encodePacked("borrower");
        layoutSegments[1] = abi.encodePacked("amount");

        // Submit query on-chain (stored in pallet)
        bytes32 queryId = prover.submitQuery(
            chainId,
            blockNumber,
            txIndex,
            layoutSegments
        );

        // Query is now visible to all prover nodes
    }

    // Prover network generates proof off-chain and submits it
    // Contract is notified via callback or event
    function onProofVerified(
        bytes32 queryId,
        bytes memory resultSegments
    ) external {
        require(msg.sender == address(prover), "Only prover");

        // Process verified data
        (address borrower, uint256 amount) = abi.decode(
            resultSegments,
            (address, uint256)
        );

        updateCreditScore(borrower, amount);
    }
}
```

**Advantages:**
- ✅ Separation of concerns: Contract doesn't handle proof generation
- ✅ Competitive: Multiple provers can compete
- ✅ Incentivized: Provers can be paid for their work

**Disadvantages:**
- ⚠️ Asynchronous: Results come later (seconds to minutes)
- ⚠️ Complexity: Requires prover network infrastructure

---

## Design Patterns and Solutions

### Recommended Approach for Different Use Cases

#### Use Case 1: High-Volume, Low-Stakes (e.g., Gaming, Social)

**Solution**: Single Trusted Relayer + Native Precompile

```javascript
// Fast, cheap, good enough for low-stakes applications
const relayer = new AutomatedRelayer({
    sourceChain: 'ethereum',
    targetChain: 'creditcoin3',
    events: ['GameScore', 'Achievement'],
    verificationMode: 'native',
});

relayer.start();
```

#### Use Case 2: High-Stakes, Trustless (e.g., DeFi, Custody)

**Solution**: STARK Proofs

```javascript
// Slower but cryptographically secure
const prover = new StarkProverService({
    sourceChain: 'ethereum',
    targetChain: 'creditcoin3',
    events: ['LargeTransfer', 'CustodyChange'],
    verificationMode: 'stark',
    minAmount: ethers.utils.parseEther('100'),
});

prover.start();
```

#### Use Case 3: Medium-Stakes, Fast (e.g., Credit Scoring, Reputation)

**Solution**: Decentralized Oracle Network + Native Precompile

```javascript
// Fast, reasonably secure
const oracleNetwork = new DecentralizedOracles({
    nodes: ['node1.example.com', 'node2.example.com', 'node3.example.com'],
    requiredSignatures: 2,
    verificationMode: 'native',
});

oracleNetwork.start();
```

### Data Flow Summary

```
┌─────────────────────────────────────────────────────────────┐
│  WHO PROVIDES BLOCK DATA?                                   │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ANSWER: An off-chain service ALWAYS                       │
│                                                              │
│  The blockchain cannot fetch data from other chains.        │
│  Someone must:                                              │
│    1. Monitor source chain for events                       │
│    2. Fetch block data via RPC                             │
│    3. Build merkle tree locally                            │
│    4. Generate proof (STARK or native)                     │
│    5. Submit to target chain                               │
│                                                              │
│  This can be:                                               │
│    - Centralized relayer (fast, simple)                    │
│    - Decentralized oracle network (secure, complex)        │
│    - User themselves (permissionless, high friction)       │
│    - Prover network (current CC3 model)                    │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Native Precompile: What It Provides

The native precompile **does not** fetch block data. It **only** verifies that the provided data is correct:

```rust
// What the precompile DOES:
fn verify_merkle_proof(
    root: Felt,              // ✅ Provided by attestation (already on-chain)
    leaf: Felt,              // ❌ Must be provided by caller
    index: u64,              // ✅ Part of query (already on-chain)
    siblings: Vec<Felt>      // ❌ Must be provided by caller (from off-chain)
) -> bool {
    // Verifies the mathematical relationship
    // Does NOT fetch any data from external chains
}
```

### Block Data Requirements

| Component | Source | Who Provides |
|-----------|--------|--------------|
| **Merkle Root** | Attestation (on-chain) | Attestation network |
| **Transaction Data** | Source chain RPC | Off-chain service |
| **Transaction Receipts** | Source chain RPC | Off-chain service |
| **Merkle Siblings** | Computed locally | Off-chain service |
| **Continuity Chain** | Attestation cache (database) | Off-chain service |
| **Query Specification** | On-chain (Prover Pallet) | User/Contract |

---

## Conclusion

### Key Takeaways

1. **Block data must always come from off-chain**: Neither STARK nor native precompiles eliminate the need for an off-chain component to fetch block data from the source chain.

2. **Native precompiles are verification only**: They verify that provided data is correct, but cannot fetch data themselves.

3. **Trade-offs remain the same**:
   - **STARK**: Trustless, universal, but slow (15 min) and expensive
   - **Native Precompile**: Fast (<100ms), cheap, but requires trust in CC3 validators

4. **Automatic event handling requires infrastructure**: To automatically generate queries from events, you need one of:
   - Centralized relayer (simple, fast)
   - Decentralized oracle network (secure, complex)
   - User-submitted proofs (permissionless, high friction)
   - Prover network (current model, flexible)

### Answering the Original Question

**Q: How does a dApp contract automatically generate a query based on an event, and how do we provide block data to create the proof?**

**A: The dApp cannot do this alone. You need an off-chain service that:**

1. **Monitors the source chain** for relevant events (e.g., loan repayments)
2. **Fetches block data** from the source chain RPC when an event occurs
3. **Builds the merkle tree** locally from the block's transactions
4. **Generates the merkle proof** for the specific transaction
5. **Retrieves the continuity chain** from the attestation cache/network
6. **Submits everything** to the target chain for verification

**With Native Precompile:**
```solidity
// Off-chain relayer does steps 1-5, then calls:
function handleEvent(
    Query memory query,
    bytes memory txData,
    MerkleProof memory proof,
    ContinuityChain memory continuity
) external {
    require(
        NativePrecompile.verifyQuery(query, txData, proof, continuity),
        "Invalid proof"
    );
    // Use verified data...
}
```

**With STARK Proof:**
```solidity
// Off-chain prover does steps 1-5, generates STARK proof, then calls:
function handleEvent(
    Query memory query,
    bytes memory starkProof
) external {
    require(
        StarkVerifier.verify(query, starkProof),
        "Invalid proof"
    );
    // Use verified data...
}
```

### Recommended Architecture

For most use cases requiring automatic event handling:

```
┌──────────────────┐
│  Source Chain    │
│  (Ethereum)      │
│                  │
│  Event Emitted   │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  Relayer Service │  ◄─── This is REQUIRED
│  (Off-chain)     │       (no way around it)
│                  │
│  - Event monitor │
│  - RPC client    │
│  - Merkle builder│
│  - Proof gen     │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  Creditcoin3     │
│                  │
│  Native          │
│  Precompile      │  ◄─── Fast verification
│                  │       (<100ms)
│  Or              │
│                  │
│  STARK           │  ◄─── Trustless
│  Verifier        │       (15 min)
└──────────────────┘
```

### Next Steps

If you want to implement automatic query generation:

1. **Choose verification method**:
   - Native precompile for speed (if trust is acceptable)
   - STARK for trustlessness (if latency is acceptable)

2. **Implement off-chain relayer**:
   - Monitor source chain events
   - Fetch block data via RPC
   - Build merkle trees and proofs
   - Submit to Creditcoin3

3. **Design trust model**:
   - Single trusted relayer (centralized)
   - Multi-sig oracle network (decentralized)
   - Open submission + incentives (permissionless)

4. **Handle edge cases**:
   - What if relayer is offline?
   - What if attestations lag behind?
   - What if source chain reorganizes?

### Further Reading

- [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) - Performance comparison
- [WHAT_IS_BEING_PROVEN.md](./WHAT_IS_BEING_PROVEN.md) - Security properties
- [QUERY_HASH_SIMPLIFICATION.md](./QUERY_HASH_SIMPLIFICATION.md) - Query processing details
