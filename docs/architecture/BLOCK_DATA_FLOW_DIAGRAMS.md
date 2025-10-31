# Block Data Flow Diagrams

**Last Updated:** January 2025

This document provides visual diagrams showing exactly how block data flows through the system for proof generation and verification.

---

## Table of Contents
1. [Current STARK-based Flow](#current-stark-based-flow)
2. [Native Precompile Flow](#native-precompile-flow)
3. [Event-Driven Automatic Query Flow](#event-driven-automatic-query-flow)
4. [Data Sources and Dependencies](#data-sources-and-dependencies)

---

## Current STARK-based Flow

### Complete End-to-End Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SOURCE CHAIN (ETHEREUM)                            │
│                                                                               │
│  Block 1000                                                                   │
│  ┌─────────────────────────────────────────────────────────────────┐        │
│  │ Transactions:                                                     │        │
│  │   [0] Transfer: Alice → Bob (10 ETH)                            │        │
│  │   [1] Swap: Carol → DEX                                         │        │
│  │   [2] Loan Repayment: Dave → Lender (1000 USDC) ◄──── TARGET   │        │
│  │   [3] NFT Mint: Eve                                             │        │
│  │   ...                                                            │        │
│  │   [99] Transfer: Frank → Grace                                  │        │
│  └─────────────────────────────────────────────────────────────────┘        │
│                                                                               │
│  ▲                                                                            │
│  │ (4) RPC Request: eth_getBlockByNumber(1000)                              │
│  │     Returns: Block header + all 100 transactions + receipts              │
└──┼───────────────────────────────────────────────────────────────────────────┘
   │
   │
   │
┌──┴────────────────────────────────────────────────────────────────────────┐
│                      PROVER SERVICE (OFF-CHAIN)                            │
│                                                                             │
│  Step 1: Monitor for Queries                                               │
│  ┌──────────────────────────────────────────────┐                         │
│  │ cc3_client.subscribe_events()                │                         │
│  │   → Receives: QuerySubmitted(                 │                         │
│  │       query_id: 0xabc123...,                 │                         │
│  │       chain_id: 1,           (Ethereum)      │                         │
│  │       block: 1000,                           │                         │
│  │       tx_index: 2,           (Loan repayment)│                         │
│  │       layout_segments: [...]                 │                         │
│  │     )                                         │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 2: Check Attestations                                                │
│  ┌──────────────────────────────────────────────┐                         │
│  │ attestation_cache.last_synced_attestation(1) │                         │
│  │   → Returns: 1005                            │                         │
│  │   ✓ Block 1000 is attested (1000 < 1005)    │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 3: Fetch Block Data via RPC                                          │
│  ┌──────────────────────────────────────────────┐                         │
│  │ eth_client.get_block(1000, encoding)         │ ────────┐               │
│  │                                               │         │ RPC Request   │
│  │ Returns OrderedBlock {                       │ ◄───────┘ (above)       │
│  │   block: Block<Transaction> {                │                         │
│  │     number: 1000,                            │                         │
│  │     timestamp: 1673456789,                   │                         │
│  │     transactions: [                          │                         │
│  │       Tx0, Tx1, Tx2, Tx3, ..., Tx99         │                         │
│  │     ]                                        │                         │
│  │   },                                         │                         │
│  │   receipts: [                                │                         │
│  │     Receipt0, Receipt1, Receipt2, ..., Receipt99                      │
│  │   ]                                          │                         │
│  │ }                                            │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 4: Build Merkle Tree Locally                                         │
│  ┌──────────────────────────────────────────────┐                         │
│  │ mt = starknet_pedersen_mmr(&block)           │                         │
│  │                                               │                         │
│  │ Builds tree from all 100 transactions:       │                         │
│  │                                               │                         │
│  │         Root (0xdef456...)                   │                         │
│  │          /              \                     │                         │
│  │      Node1              Node2                │                         │
│  │      /    \            /    \                │                         │
│  │   Node3  Node4     Node5  Node6             │                         │
│  │   /  \   /  \      /  \   /  \              │                         │
│  │  Tx0 Tx1 Tx2 Tx3  ...      Tx98 Tx99        │                         │
│  │           ↑                                   │                         │
│  │           └── Target transaction (index 2)   │                         │
│  │                                               │                         │
│  │ merkle_proof = mt.generate_proof(2)          │                         │
│  │   → siblings: [Tx3_hash, Node3_hash, ...]   │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 5: Get Continuity Chain                                              │
│  ┌──────────────────────────────────────────────┐                         │
│  │ fragment::get_for_query(...)                 │                         │
│  │                                               │                         │
│  │ Retrieves attestations from DB:              │                         │
│  │   Block 990 → digest: 0x111...               │                         │
│  │   Block 991 → digest: 0x222...               │                         │
│  │   ...                                         │                         │
│  │   Block 1000 → digest: 0xaaa...              │                         │
│  │                                               │                         │
│  │ Verifies chain continuity:                   │                         │
│  │   hash(digest_990) == digest_991 ✓           │                         │
│  │   hash(digest_991) == digest_992 ✓           │                         │
│  │   ...                                         │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 6: Prepare Cairo Input                                               │
│  ┌──────────────────────────────────────────────┐                         │
│  │ QueryProver::new(                             │                         │
│  │   block_number: 1000,                        │                         │
│  │   merkle_proof: [siblings...],               │                         │
│  │   subject_bytes: Tx2_data,                   │                         │
│  │   query: query_serializable,                 │                         │
│  │   continuity_chain: fragment_blocks,         │                         │
│  │   ...                                         │                         │
│  │ )                                             │                         │
│  │                                               │                         │
│  │ Writes to: /var/tmp/.../program_input.json   │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 7: Run Cairo Verifier                                                │
│  ┌──────────────────────────────────────────────┐                         │
│  │ verify_merkle_proof.cairo                    │                         │
│  │                                               │                         │
│  │ Executes:                                     │                         │
│  │   1. Verify merkle proof                     │                         │
│  │   2. Verify continuity chain                 │                         │
│  │   3. Extract query data segments             │                         │
│  │                                               │                         │
│  │ Outputs: /var/tmp/.../output.txt             │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 8: Generate STARK Proof                                              │
│  ┌──────────────────────────────────────────────┐                         │
│  │ cpu_air_prover (Stone Prover)                │                         │
│  │                                               │                         │
│  │ ⏱️  Takes ~15 minutes                         │                         │
│  │ 💻 Uses 32 CPU cores @ 100%                  │                         │
│  │ 💾 Consumes 2-4 GB RAM                       │                         │
│  │                                               │                         │
│  │ Generates: proof.json (~500 KB)              │                         │
│  │   - STARK proof of Cairo execution           │                         │
│  │   - Public outputs (query results)           │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        ▼                                                    │
│           Submit proof to Creditcoin3...                                   │
└─────────────────────────────────────────────────────────────────────────────┘
                         │
                         │ (9) Submit Transaction
                         │
┌────────────────────────▼─────────────────────────────────────────────────┐
│                    CREDITCOIN3 CHAIN (ON-CHAIN)                           │
│                                                                            │
│  Step 9: Proof Submission                                                 │
│  ┌────────────────────────────────────────────┐                          │
│  │ prover_contract.submitProof(               │                          │
│  │   query_id: 0xabc123...,                   │                          │
│  │   proof: [500 KB STARK proof bytes]        │                          │
│  │ )                                           │                          │
│  └────────────────────────────────────────────┘                          │
│                    │                                                       │
│                    │                                                       │
│  Step 10: Call Verifier Precompile                                        │
│  ┌────────────────────────────────────────────┐                          │
│  │ Precompile 0x0Be9                          │                          │
│  │                                             │                          │
│  │ verifier_core::run_verifier(               │                          │
│  │   proof,                                   │                          │
│  │   query,                                   │                          │
│  │   metadata                                 │                          │
│  │ )                                           │                          │
│  └────────────────────────────────────────────┘                          │
│                    │                                                       │
│                    │                                                       │
│  Step 11: STARK Verification                                              │
│  ┌────────────────────────────────────────────┐                          │
│  │ Stone Verifier (cpu_air_verifier)         │                          │
│  │                                             │                          │
│  │ Cryptographically verifies:                │                          │
│  │   ✓ Cairo program executed correctly       │                          │
│  │   ✓ Merkle proof is valid                 │                          │
│  │   ✓ Continuity chain is valid             │                          │
│  │                                             │                          │
│  │ ⚡ Takes ~2-5M gas                         │                          │
│  │ ⏱️  Takes ~2-5 seconds                     │                          │
│  └────────────────────────────────────────────┘                          │
│                    │                                                       │
│                    │                                                       │
│  Step 12: Validate Continuity Digest                                      │
│  ┌────────────────────────────────────────────┐                          │
│  │ Check Attestations Pallet                  │                          │
│  │                                             │                          │
│  │ continuity_digest = extract_from_proof()   │                          │
│  │ block_number = calculate_block_number()    │                          │
│  │                                             │                          │
│  │ attestation = get_attestation_by_digest()  │                          │
│  │ require(attestation.block == block_number) │                          │
│  └────────────────────────────────────────────┘                          │
│                    │                                                       │
│                    │                                                       │
│  Step 13: Return Results                                                  │
│  ┌────────────────────────────────────────────┐                          │
│  │ ProofVerified event emitted                │                          │
│  │                                             │                          │
│  │ Result segments available:                 │                          │
│  │   - Borrower: Dave                         │                          │
│  │   - Amount: 1000 USDC                      │                          │
│  │   - Verified: ✓                            │                          │
│  │                                             │                          │
│  │ Smart contracts can now use this data!     │                          │
│  └────────────────────────────────────────────┘                          │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

### Key Observation

**Block data flows from Source Chain → Prover Service → STARK Proof → On-chain Verification**

The prover service is the **critical component** that bridges the source chain and the target chain.

---

## Native Precompile Flow

### How Native Precompiles Change the Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SOURCE CHAIN (ETHEREUM)                            │
│                                                                               │
│  Block 1000: Loan Repayment Transaction                                      │
│                                                                               │
│  ▲                                                                            │
│  │ (2) Still needs RPC request!                                              │
│  │     Native precompile doesn't eliminate this                              │
└──┼───────────────────────────────────────────────────────────────────────────┘
   │
   │
┌──┴────────────────────────────────────────────────────────────────────────┐
│                  RELAYER SERVICE (OFF-CHAIN)                               │
│                  [Can be same as Prover Service]                           │
│                                                                             │
│  Step 1-5: Same as STARK flow                                              │
│  ┌──────────────────────────────────────────────┐                         │
│  │ 1. Monitor queries                           │                         │
│  │ 2. Check attestations                        │                         │
│  │ 3. Fetch block data via RPC     ◄─── STILL REQUIRED                   │
│  │ 4. Build merkle tree locally                 │                         │
│  │ 5. Get continuity chain                      │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        │                                                    │
│  Step 6: Prepare Proof Data (MUCH FASTER)                                  │
│  ┌──────────────────────────────────────────────┐                         │
│  │ merkle_proof = {                             │                         │
│  │   root: 0xdef456...,                         │                         │
│  │   leaf: Tx2_hash,                            │                         │
│  │   index: 2,                                  │                         │
│  │   siblings: [Tx3_hash, Node3_hash, ...]     │                         │
│  │ }                                             │                         │
│  │                                               │                         │
│  │ continuity_chain = {                         │                         │
│  │   blocks: [Block990, ..., Block1000],       │                         │
│  │   attestations: [...]                        │                         │
│  │ }                                             │                         │
│  │                                               │                         │
│  │ ⏱️  Takes <100ms (vs 15 min for STARK)       │                         │
│  └──────────────────────────────────────────────┘                         │
│                        │                                                    │
│                        ▼                                                    │
│           Submit to native precompile...                                   │
└─────────────────────────────────────────────────────────────────────────────┘
                         │
                         │ (7) Submit Transaction
                         │
┌────────────────────────▼─────────────────────────────────────────────────┐
│                    CREDITCOIN3 CHAIN (ON-CHAIN)                           │
│                                                                            │
│  Step 7: Direct Verification Call                                         │
│  ┌────────────────────────────────────────────┐                          │
│  │ contract.verifyTransaction(                │                          │
│  │   query: {                                 │                          │
│  │     chainId: 1,                            │                          │
│  │     block: 1000,                           │                          │
│  │     txIndex: 2                             │                          │
│  │   },                                       │                          │
│  │   txData: [Tx2 bytes],                     │                          │
│  │   merkleProof: {                           │                          │
│  │     root: 0xdef456...,                     │                          │
│  │     siblings: [...]                        │                          │
│  │   },                                       │                          │
│  │   continuityChain: {                       │                          │
│  │     blocks: [...],                         │                          │
│  │     attestations: [...]                    │                          │
│  │   }                                        │                          │
│  │ )                                           │                          │
│  └────────────────────────────────────────────┘                          │
│                    │                                                       │
│                    │                                                       │
│  Step 8: Native Precompile Verification                                   │
│  ┌────────────────────────────────────────────┐                          │
│  │ Precompile 0x0BeA (example address)        │                          │
│  │                                             │                          │
│  │ verifyMerkleProof():                       │                          │
│  │   current = hash(txData)                   │                          │
│  │   for sibling in siblings:                 │                          │
│  │     current = pedersen(current, sibling)   │                          │
│  │   require(current == root)                 │                          │
│  │                                             │                          │
│  │ verifyContinuity():                        │                          │
│  │   for block in continuityChain:            │                          │
│  │     require(hash(prev) == current.digest)  │                          │
│  │   require(attestations valid)              │                          │
│  │                                             │                          │
│  │ ⚡ Takes ~1.5-2.5M gas                      │                          │
│  │ ⏱️  Takes ~100ms                            │                          │
│  └────────────────────────────────────────────┘                          │
│                    │                                                       │
│                    │                                                       │
│  Step 9: Return Results Immediately                                       │
│  ┌────────────────────────────────────────────┐                          │
│  │ return (true, extractedData)               │                          │
│  │                                             │                          │
│  │ Smart contract uses data in SAME txn!      │                          │
│  └────────────────────────────────────────────┘                          │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

### Comparison

| Step | STARK | Native Precompile |
|------|-------|-------------------|
| **Fetch block data** | ✅ Required (RPC) | ✅ Required (RPC) |
| **Build merkle tree** | ✅ Required (off-chain) | ✅ Required (off-chain) |
| **Get continuity** | ✅ Required (DB) | ✅ Required (DB) |
| **Proof generation** | 15 minutes (Cairo+Stone) | <100ms (just prep data) |
| **Proof size** | 500 KB | 20-50 KB |
| **On-chain verification** | 2-5M gas, 2-5 sec | 1.5-2.5M gas, 100ms |
| **Trust model** | Trustless | Trusts CC3 validators |
| **Cross-chain** | ✅ Universal | ❌ CC3 only |

**Key Insight:** Native precompiles don't eliminate the need for off-chain block fetching. They only make verification faster and cheaper.

---

## Event-Driven Automatic Query Flow

### Pattern: Relayer Service

This shows how to automatically handle events from a source chain.

```
┌─────────────────────────────────────────────────────────────────────┐
│                  SOURCE CHAIN (ETHEREUM)                             │
│                                                                       │
│  Smart Contract: LoanManager                                         │
│  ┌──────────────────────────────────────────────────────┐           │
│  │ function repayLoan(uint256 loanId) external {        │           │
│  │     require(msg.sender == borrower);                 │           │
│  │     // ... payment logic ...                         │           │
│  │                                                        │           │
│  │     emit LoanRepaid(                                 │  ◄──┐     │
│  │         loanId,                                      │     │     │
│  │         msg.sender,                                  │     │     │
│  │         amount,                                      │     │     │
│  │         block.timestamp                              │     │     │
│  │     );                                                │     │     │
│  │ }                                                     │     │     │
│  └──────────────────────────────────────────────────────┘     │     │
│                                                                │     │
│  Event Log: LoanRepaid(                                       │     │
│    loanId: 42,                                                │     │
│    borrower: 0x1234...,                                       │     │
│    amount: 1000 USDC,                                         │     │
│    timestamp: 1673456789                                      │     │
│  )                                                             │     │
│  Block: 1000, Tx Index: 2                                     │     │
└────────────────────────────────────────────────────────────────┼─────┘
                                                                 │
                                                                 │ (1) Event
                                                                 │     Detected
┌────────────────────────────────────────────────────────────────▼─────┐
│                     RELAYER SERVICE (OFF-CHAIN)                       │
│                                                                        │
│  Component 1: Event Monitor                                           │
│  ┌──────────────────────────────────────────────────────┐            │
│  │ const provider = new ethers.providers                │            │
│  │   .JsonRpcProvider(ETH_RPC);                         │            │
│  │                                                        │            │
│  │ const loanContract = new ethers.Contract(            │            │
│  │   LOAN_ADDRESS, LOAN_ABI, provider                   │            │
│  │ );                                                    │            │
│  │                                                        │            │
│  │ loanContract.on('LoanRepaid', async (               │            │
│  │   loanId, borrower, amount, timestamp, event        │            │
│  │ ) => {                                               │            │
│  │   await handleRepayment(event);                     │            │
│  │ });                                                   │            │
│  └──────────────────────────────────────────────────────┘            │
│                         │                                             │
│                         │                                             │
│  Component 2: Block Data Fetcher                                     │
│  ┌──────────────────────────────────────────────────────┐            │
│  │ async function handleRepayment(event) {              │            │
│  │   const blockNum = event.blockNumber;  // 1000      │            │
│  │   const txIndex = await getTxIndex(event);  // 2    │            │
│  │                                                        │            │
│  │   // Fetch block data                                │            │
│  │   const block = await provider.getBlock(blockNum);  │            │
│  │   const tx = await provider.getTransaction(         │            │
│  │     event.transactionHash                           │            │
│  │   );                                                 │            │
│  │   const receipt = await provider                    │            │
│  │     .getTransactionReceipt(                         │            │
│  │       event.transactionHash                         │            │
│  │     );                                               │            │
│  │                                                        │            │
│  │   return { blockNum, txIndex, tx, receipt };       │            │
│  │ }                                                     │            │
│  └──────────────────────────────────────────────────────┘            │
│                         │                                             │
│                         │                                             │
│  Component 3: Merkle Tree Builder                                    │
│  ┌──────────────────────────────────────────────────────┐            │
│  │ function buildMerkleProof(block, txIndex) {          │            │
│  │   // Get all transactions in block                   │            │
│  │   const leaves = block.transactions.map(tx => {     │            │
│  │     return keccak256(encodeTx(tx));                 │            │
│  │   });                                                │            │
│  │                                                        │            │
│  │   // Build merkle tree                               │            │
│  │   const tree = new MerkleTree(leaves,               │            │
│  │     pedersenHash                                     │            │
│  │   );                                                 │            │
│  │                                                        │            │
│  │   // Generate proof for target transaction           │            │
│  │   const proof = tree.getProof(txIndex);             │            │
│  │   const root = tree.getRoot();                      │            │
│  │                                                        │            │
│  │   return { proof, root };                           │            │
│  │ }                                                     │            │
│  └──────────────────────────────────────────────────────┘            │
│                         │                                             │
│                         │                                             │
│  Component 4: Attestation Fetcher                                    │
│  ┌──────────────────────────────────────────────────────┐            │
│  │ async function getContinuityChain(blockNum) {        │            │
│  │   // Query local attestation cache/DB                │            │
│  │   const startBlock = blockNum - 10;                 │            │
│  │   const endBlock = blockNum;                        │            │
│  │                                                        │            │
│  │   const attestations = await db.query(              │            │
│  │     'SELECT * FROM attestations ' +                 │            │
│  │     'WHERE block_number BETWEEN ? AND ?',           │            │
│  │     [startBlock, endBlock]                          │            │
│  │   );                                                 │            │
│  │                                                        │            │
│  │   return buildContinuityChain(attestations);        │            │
│  │ }                                                     │            │
│  └──────────────────────────────────────────────────────┘            │
│                         │                                             │
│                         │                                             │
│  Component 5: Proof Generator & Submitter                            │
│  ┌──────────────────────────────────────────────────────┐            │
│  │ async function submitProof(eventData) {              │            │
│  │   const { blockNum, txIndex, tx, receipt } =        │            │
│  │     eventData;                                       │            │
│  │                                                        │            │
│  │   const { proof, root } =                           │            │
│  │     buildMerkleProof(block, txIndex);               │            │
│  │                                                        │            │
│  │   const continuityChain =                           │            │
│  │     await getContinuityChain(blockNum);             │            │
│  │                                                        │            │
│  │   // OPTION A: Use native precompile                │            │
│  │   const cc3Provider = new ethers.providers          │            │
│  │     .JsonRpcProvider(CC3_RPC);                      │            │
│  │                                                        │            │
│  │   const contract = new ethers.Contract(             │            │
│  │     CREDIT_CONTRACT, ABI, cc3Wallet                 │            │
│  │   );                                                 │            │
│  │                                                        │            │
│  │   const txData = encodeTxData(tx, receipt);         │            │
│  │                                                        │            │
│  │   await contract.verifyRepayment(                   │            │
│  │     {                                                │            │
│  │       chainId: 1,                                   │            │
│  │       block: blockNum,                              │            │
│  │       txIndex: txIndex                              │            │
│  │     },                                               │            │
│  │     txData,                                         │            │
│  │     proof,                                          │            │
│  │     continuityChain                                 │            │
│  │   );                                                 │            │
│  │                                                        │            │
│  │   // OPTION B: Submit query for STARK proving       │            │
│  │   // await proverPallet.submitQuery(...)            │            │
│  │ }                                                     │            │
│  └──────────────────────────────────────────────────────┘            │
│                         │                                             │
│                         ▼                                             │
│           Submit to Creditcoin3...                                   │
└────────────────────────────────────────────────────────────────────────┘
                          │
                          │ (2) Transaction with proof data
                          │
┌─────────────────────────▼──────────────────────────────────────────────┐
│                    CREDITCOIN3 CHAIN (ON-CHAIN)                         │
│                                                                          │
│  Smart Contract: CreditRatingSystem                                     │
│  ┌──────────────────────────────────────────────────────┐              │
│  │ function verifyRepayment(                            │              │
│  │     Query memory query,                              │              │
│  │     bytes memory txData,                             │              │
│  │     MerkleProof memory proof,                        │              │
│  │     ContinuityChain memory continuity                │              │
│  │ ) external {                                         │              │
│  │     // Only trusted relayer can submit               │              │
│  │     require(                                          │              │
│  │         msg.sender == trustedRelayer,                │              │
│  │         "Unauthorized"                                │              │
│  │     );                                                │              │
│  │                                                        │              │
│  │     // Verify using native precompile                │              │
│  │     (bool valid, bytes memory data) =                │              │
│  │         NativePrecompile.verifyQuery(                │              │
│  │             query,                                    │              │
│  │             txData,                                   │              │
│  │             proof,                                    │              │
│  │             continuity                                │              │
│  │         );                                            │              │
│  │                                                        │              │
│  │     require(valid, "Invalid proof");                 │              │
│  │                                                        │              │
│  │     // Decode verified data                          │              │
│  │     (address borrower, uint256 amount) =             │              │
│  │         abi.decode(data, (address, uint256));        │              │
│  │                                                        │              │
│  │     // Update credit score                           │              │
│  │     creditScores[borrower] += calculateBonus(amount);│              │
│  │                                                        │              │
│  │     emit CreditScoreUpdated(borrower, amount);       │              │
│  │ }                                                     │              │
│  └──────────────────────────────────────────────────────┘              │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

### Timeline Comparison

**With Native Precompile:**
```
Event Emitted (Ethereum)
    ↓ <1 second (event detection)
Relayer Detects Event
    ↓ <1 second (RPC fetch)
Fetch Block Data
    ↓ <100ms (local computation)
Build Merkle Proof
    ↓ <100ms (DB query)
Get Continuity Chain
    ↓ <1 second (transaction confirmation)
Submit to CC3
    ↓ Immediate (same transaction)
Verification Complete
─────────────────────
Total: ~3-5 seconds
```

**With STARK Proof:**
```
Event Emitted (Ethereum)
    ↓ <1 second
Relayer Detects Event
    ↓ <1 second
Fetch Block Data
    ↓ <1 second
Build Proof Input
    ↓ 15 minutes (STARK generation)
Generate STARK Proof
    ↓ <1 second
Submit to CC3
    ↓ 2-5 seconds (STARK verification)
Verification Complete
─────────────────────
Total: ~15-20 minutes
```

---

## Data Sources and Dependencies

### Where Each Piece of Data Comes From

```
┌─────────────────────────────────────────────────────────────────┐
│                    DATA SOURCE MAPPING                           │
└─────────────────────────────────────────────────────────────────┘

1. QUERY SPECIFICATION
   ┌────────────────────────────────────┐
   │ Source: On-chain (Prover Pallet)   │
   │ Contains:                          │
   │   - Chain ID                       │
   │   - Block number                   │
   │   - Transaction index              │
   │   - Layout segments                │
   │                                    │
   │ Who provides: User or Contract     │
   └────────────────────────────────────┘

2. BLOCK DATA (Critical - requires RPC)
   ┌────────────────────────────────────┐
   │ Source: Source Chain RPC Endpoint  │
   │ Method: eth_getBlockByNumber       │
   │ Contains:                          │
   │   - Block header                   │
   │   - All transactions               │
   │   - All transaction receipts       │
   │   - Block metadata                 │
   │                                    │
   │ Who provides: Off-chain service    │
   │ Why needed: Build merkle tree      │
   └────────────────────────────────────┘

3. MERKLE ROOT
   ┌────────────────────────────────────┐
   │ Source: Attestation Network        │
   │ Stored: On-chain (Attestations)    │
   │ Contains:                          │
   │   - Merkle root of block           │
   │   - Block number                   │
   │   - Validator signatures           │
   │                                    │
   │ Who provides: Attestors (trusted)  │
   │ Why needed: Verify merkle proof    │
   └────────────────────────────────────┘

4. MERKLE SIBLINGS (Proof Path)
   ┌────────────────────────────────────┐
   │ Source: Computed locally           │
   │ Method: merkle_tree.generate_proof │
   │ Contains:                          │
   │   - Sibling hashes                 │
   │   - Path indices                   │
   │                                    │
   │ Who provides: Off-chain service    │
   │ Why needed: Prove inclusion        │
   │ Depends on: Block data (step 2)    │
   └────────────────────────────────────┘

5. CONTINUITY CHAIN
   ┌────────────────────────────────────┐
   │ Source: Attestation Cache/DB       │
   │ Contains:                          │
   │   - Block digests                  │
   │   - Chain of attestations          │
   │   - Checkpoint references          │
   │                                    │
   │ Who provides: Off-chain service    │
   │ Why needed: Prove finality         │
   └────────────────────────────────────┘

6. TRANSACTION DATA
   ┌────────────────────────────────────┐
   │ Source: Extracted from block       │
   │ Method: block.transactions[index]  │
   │ Contains:                          │
   │   - Transaction bytes              │
   │   - Receipt bytes                  │
   │   - Decoded fields                 │
   │                                    │
   │ Who provides: Off-chain service    │
   │ Why needed: What we're proving     │
   │ Depends on: Block data (step 2)    │
   └────────────────────────────────────┘
```

### Dependency Graph

```
                    ┌─────────────────┐
                    │  User/Contract  │
                    │  Submits Query  │
                    └────────┬────────┘
                             │
                             ▼
                    ┌─────────────────┐
                    │ Query Stored    │
                    │ On-Chain        │
                    └────────┬────────┘
                             │
                             ▼
            ┌────────────────────────────────┐
            │   Off-chain Service Required   │
            │   (No way to avoid this!)      │
            └────────┬───────────────────────┘
                     │
                     ├──────────────────────────────┐
                     │                              │
                     ▼                              ▼
         ┌───────────────────┐        ┌────────────────────┐
         │ Fetch Block Data  │        │ Fetch Attestations │
         │ via RPC           │        │ from DB/Network    │
         └─────────┬─────────┘        └──────────┬─────────┘
                   │                              │
                   ▼                              │
         ┌───────────────────┐                   │
         │ Build Merkle Tree │                   │
         └─────────┬─────────┘                   │
                   │                              │
                   ▼                              │
         ┌───────────────────┐                   │
         │ Generate Proof    │                   │
         └─────────┬─────────┘                   │
                   │                              │
                   └──────────┬───────────────────┘
                              │
                              ▼
                   ┌─────────────────────┐
                   │ Submit to Target    │
                   │ Chain for           │
                   │ Verification        │
                   └─────────────────────┘
```

### The Unavoidable Off-chain Requirement

```
╔═══════════════════════════════════════════════════════════════╗
║                                                               ║
║  WHY OFF-CHAIN COMPONENT IS ALWAYS REQUIRED                  ║
║                                                               ║
║  Blockchain Limitation:                                      ║
║  Smart contracts CANNOT make HTTP requests to external       ║
║  chains. They cannot fetch block data from Ethereum,         ║
║  Polygon, etc.                                               ║
║                                                               ║
║  Even with native precompiles:                               ║
║  - Precompile can VERIFY data is correct                     ║
║  - Precompile CANNOT fetch data from other chains            ║
║  - Someone must provide the data as input                    ║
║                                                               ║
║  Solutions:                                                   ║
║  1. Centralized relayer (fast, simple, centralized)          ║
║  2. Decentralized oracles (secure, complex, expensive)       ║
║  3. Prover network (flexible, asynchronous)                  ║
║  4. User submission (permissionless, high friction)          ║
║                                                               ║
║  All solutions require an off-chain component that:          ║
║  - Has RPC access to source chain                           ║
║  - Can compute merkle trees                                  ║
║  - Can submit transactions to target chain                   ║
║                                                               ║
╚═══════════════════════════════════════════════════════════════╝
```

### Native Precompile vs STARK: What Changes

| Component | STARK | Native Precompile | Can Be Eliminated? |
|-----------|-------|-------------------|-------------------|
| **RPC Access** | ✅ Required | ✅ Required | ❌ No |
| **Fetch Block Data** | ✅ Required | ✅ Required | ❌ No |
| **Build Merkle Tree** | ✅ Required | ✅ Required | ❌ No |
| **Get Continuity** | ✅ Required | ✅ Required | ❌ No |
| **Cairo Execution** | ✅ Required (15 min) | ❌ Not needed | ✅ Yes (saved!) |
| **STARK Generation** | ✅ Required (15 min) | ❌ Not needed | ✅ Yes (saved!) |
| **On-chain Verification** | STARK verifier (2-5M gas) | Native precompile (1.5-2.5M gas) | ❌ No (but faster) |
| **Trust Model** | Trustless | Trusts validators | Different, not eliminated |

**Key Insight:** Native precompiles eliminate the expensive proof generation steps, but cannot eliminate the need to fetch block data from external chains.

---

## Conclusion

### Critical Understanding

**Both STARK and Native Precompile approaches require:**
1. ✅ Off-chain service to monitor events
2. ✅ RPC access to source chain
3. ✅ Local merkle tree computation
4. ✅ Attestation data retrieval
5. ✅ Transaction submission to target chain

**The difference is only in proof generation and verification:**
- **STARK**: Generates cryptographic proof (15 min) → Universal trustless verification
- **Native**: Skips proof generation (<100ms) → Fast but trusts CC3 validators

### For Automatic Event Handling

You **must** implement an off-chain component. There is no way around this limitation. Choose your pattern based on requirements:

- **Speed critical** → Native precompile + centralized relayer
- **Trust critical** → STARK + decentralized oracles
- **Cost critical** → Native precompile + simple relayer
- **Flexibility** → Hybrid approach (both options available)
