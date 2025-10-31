# Automatic Query Generation from Contract Events: Practical Guide

**Last Updated:** January 2025

## Overview

This guide provides step-by-step instructions for implementing automatic query generation and verification when events occur on source chains (e.g., Ethereum loan repayment events).

---

## Table of Contents
1. [Problem Statement](#problem-statement)
2. [Architecture Options](#architecture-options)
3. [Implementation Guide](#implementation-guide)
4. [Code Examples](#code-examples)
5. [Deployment Guide](#deployment-guide)
6. [FAQ](#faq)

---

## Problem Statement

### The Challenge

You have a smart contract on Ethereum that emits events (e.g., `LoanRepaid`), and you want a smart contract on Creditcoin3 to automatically verify and react to these events.

**Example Scenario:**
```solidity
// On Ethereum
contract LoanManager {
    event LoanRepaid(
        uint256 indexed loanId,
        address indexed borrower,
        uint256 amount,
        uint256 timestamp
    );

    function repayLoan(uint256 loanId) external {
        // ... payment logic ...
        emit LoanRepaid(loanId, msg.sender, amount, block.timestamp);
    }
}

// On Creditcoin3 - You want this to happen automatically
contract CreditRating {
    function updateScoreOnRepayment(
        address borrower,
        uint256 amount
    ) external {
        // This should be triggered automatically when LoanRepaid event occurs
        creditScores[borrower] += calculateBonus(amount);
    }
}
```

### The Question

**Q: How do I provide block data to the native precompile to verify the event?**

**A: You need an off-chain service (relayer) that:**
1. Monitors Ethereum for `LoanRepaid` events
2. Fetches the block data containing the event
3. Builds a Merkle proof for the transaction
4. Submits the proof to Creditcoin3 for verification

**There is no way to do this entirely on-chain.** Blockchains cannot make HTTP requests to other blockchains.

---

## Architecture Options

### Option 1: Centralized Relayer (Recommended for Most Cases)

**Best for:** Production applications, fast response times, cost-sensitive

```
┌──────────────┐
│  Ethereum    │
│  Event Fire  │
└──────┬───────┘
       │
       ▼
┌──────────────────┐
│  Relayer Service │  ← You run this
│  (Node.js/Rust)  │
└──────┬───────────┘
       │
       ▼
┌──────────────┐
│ Creditcoin3  │
│ Verification │
└──────────────┘
```

**Pros:**
- ✅ Fast (3-5 seconds end-to-end)
- ✅ Simple to implement
- ✅ Low cost
- ✅ Easy to maintain

**Cons:**
- ⚠️ Single point of failure
- ⚠️ Requires trust in relayer operator
- ⚠️ Relayer must stay online

### Option 2: Decentralized Oracle Network

**Best for:** High-stakes applications, maximum security

```
┌──────────────┐
│  Ethereum    │
│  Event Fire  │
└──────┬───────┘
       │
       ├────────┬────────┐
       ▼        ▼        ▼
   ┌─────┐  ┌─────┐  ┌─────┐
   │Node1│  │Node2│  │Node3│
   └──┬──┘  └──┬──┘  └──┬──┘
      │        │        │
      └────────┼────────┘
               ▼
       ┌───────────────┐
       │ Consensus     │
       │ (2-of-3 sigs) │
       └───────┬───────┘
               ▼
       ┌───────────────┐
       │ Creditcoin3   │
       └───────────────┘
```

**Pros:**
- ✅ Decentralized
- ✅ No single point of failure
- ✅ Higher security

**Cons:**
- ⚠️ More complex
- ⚠️ Higher gas costs (signature verification)
- ⚠️ Slower (must wait for consensus)

### Option 3: User-Submitted Proofs

**Best for:** Fully permissionless systems

```
┌──────────────┐
│  Ethereum    │
│  User Action │
└──────────────┘
       │
       ▼
┌──────────────────┐
│  User's Browser  │
│  or Wallet       │
└──────┬───────────┘
       │
       ▼
┌──────────────┐
│ Creditcoin3  │
│ Verification │
└──────────────┘
```

**Pros:**
- ✅ Fully decentralized
- ✅ No trusted third party
- ✅ Permissionless

**Cons:**
- ⚠️ Poor UX (users must take extra steps)
- ⚠️ Users pay gas costs
- ⚠️ Requires user to run/access proof generation

### Option 4: Hybrid (Current Creditcoin3 Model)

**Best for:** Flexibility, supporting both STARK and native precompiles

```
┌──────────────┐
│ Event / User │
│ Submits Query│
└──────┬───────┘
       │
       ▼
┌──────────────────┐
│ Query Stored     │
│ On-Chain         │
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│ Prover Network   │  ← Multiple provers compete
│ Generates Proof  │
└──────┬───────────┘
       │
       ▼
┌──────────────┐
│ Creditcoin3  │
│ Verification │
└──────────────┘
```

**Pros:**
- ✅ Separation of concerns
- ✅ Competitive prover market
- ✅ Supports both STARK and native precompiles

**Cons:**
- ⚠️ Asynchronous (results come later)
- ⚠️ Requires infrastructure

---

## Implementation Guide

### Step 1: Choose Your Architecture

For this guide, we'll implement **Option 1: Centralized Relayer** as it's the most practical starting point.

### Step 2: Set Up Your Environment

**Prerequisites:**
- Node.js 18+ or Rust 1.70+
- Access to Ethereum RPC (Infura, Alchemy, or your own node)
- Access to Creditcoin3 RPC
- Private key for relayer account on Creditcoin3

**Install Dependencies (Node.js):**
```bash
npm install ethers@6 dotenv
```

**Install Dependencies (Rust):**
```toml
[dependencies]
ethers = "2.0"
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
```

### Step 3: Implement Event Monitoring

Create a service that monitors the source chain for events.

### Step 4: Implement Block Data Fetching

When an event is detected, fetch the complete block data.

### Step 5: Implement Merkle Tree Building

Build a Merkle tree from the block's transactions.

### Step 6: Implement Continuity Chain Fetching

Retrieve attestation data from the attestation network.

### Step 7: Implement Proof Submission

Submit the proof to Creditcoin3 for verification.

---

## Code Examples

### Example 1: Complete Relayer Service (Node.js)

```javascript
// relayer.js
require('dotenv').config();
const { ethers } = require('ethers');
const { MerkleTree } = require('./merkle'); // Your merkle implementation

// Configuration
const ETH_RPC_URL = process.env.ETH_RPC_URL;
const CC3_RPC_URL = process.env.CC3_RPC_URL;
const RELAYER_PRIVATE_KEY = process.env.RELAYER_PRIVATE_KEY;

const LOAN_CONTRACT_ADDRESS = '0x...'; // Ethereum loan contract
const CREDIT_CONTRACT_ADDRESS = '0x...'; // Creditcoin3 credit contract

// ABIs
const LOAN_ABI = [
    'event LoanRepaid(uint256 indexed loanId, address indexed borrower, uint256 amount, uint256 timestamp)'
];

const CREDIT_ABI = [
    'function verifyRepayment(tuple(uint64 chainId, uint64 blockNumber, uint64 txIndex) query, bytes txData, tuple(bytes32 root, bytes32[] siblings) proof, tuple(uint64[] blockNumbers, bytes32[] digests) continuity) external'
];

class RelayerService {
    constructor() {
        // Ethereum provider
        this.ethProvider = new ethers.JsonRpcProvider(ETH_RPC_URL);
        this.loanContract = new ethers.Contract(
            LOAN_CONTRACT_ADDRESS,
            LOAN_ABI,
            this.ethProvider
        );

        // Creditcoin3 provider
        this.cc3Provider = new ethers.JsonRpcProvider(CC3_RPC_URL);
        this.cc3Wallet = new ethers.Wallet(RELAYER_PRIVATE_KEY, this.cc3Provider);
        this.creditContract = new ethers.Contract(
            CREDIT_CONTRACT_ADDRESS,
            CREDIT_ABI,
            this.cc3Wallet
        );

        // Database for attestation cache (simplified)
        this.attestationCache = new Map();
    }

    async start() {
        console.log('🚀 Starting relayer service...');

        // Listen for loan repayment events
        this.loanContract.on('LoanRepaid', async (loanId, borrower, amount, timestamp, event) => {
            console.log(`📝 Loan repayment detected:`);
            console.log(`   Loan ID: ${loanId}`);
            console.log(`   Borrower: ${borrower}`);
            console.log(`   Amount: ${ethers.formatEther(amount)} ETH`);
            console.log(`   Block: ${event.blockNumber}`);

            try {
                await this.handleRepayment(event);
                console.log('✅ Successfully processed repayment');
            } catch (error) {
                console.error('❌ Failed to process repayment:', error);
            }
        });

        console.log('✅ Relayer service started');
        console.log(`   Monitoring: ${LOAN_CONTRACT_ADDRESS}`);
    }

    async handleRepayment(event) {
        // Step 1: Get transaction index
        const block = await this.ethProvider.getBlock(event.blockNumber);
        const txIndex = block.transactions.indexOf(event.transactionHash);

        console.log(`   Transaction index: ${txIndex}`);

        // Step 2: Fetch full block data
        const blockData = await this.fetchBlockData(event.blockNumber);

        // Step 3: Build merkle proof
        const merkleProof = await this.buildMerkleProof(blockData, txIndex);

        // Step 4: Get continuity chain
        const continuityChain = await this.getContinuityChain(event.blockNumber);

        // Step 5: Get transaction data
        const tx = await this.ethProvider.getTransaction(event.transactionHash);
        const receipt = await this.ethProvider.getTransactionReceipt(event.transactionHash);
        const txData = this.encodeTxData(tx, receipt);

        // Step 6: Submit to Creditcoin3
        await this.submitProof(event.blockNumber, txIndex, txData, merkleProof, continuityChain);
    }

    async fetchBlockData(blockNumber) {
        console.log(`🔍 Fetching block data for block ${blockNumber}...`);

        const block = await this.ethProvider.getBlock(blockNumber, true);
        if (!block) {
            throw new Error(`Block ${blockNumber} not found`);
        }

        // Fetch all transaction receipts in parallel
        const receiptPromises = block.transactions.map(tx =>
            this.ethProvider.getTransactionReceipt(tx.hash)
        );
        const receipts = await Promise.all(receiptPromises);

        return {
            block,
            receipts
        };
    }

    async buildMerkleProof(blockData, txIndex) {
        console.log(`🌳 Building merkle tree...`);

        // Create leaves from transactions and receipts
        const leaves = blockData.block.transactions.map((tx, i) => {
            const receipt = blockData.receipts[i];
            // Combine transaction and receipt data
            const combined = ethers.concat([
                ethers.toUtf8Bytes(JSON.stringify(tx)),
                ethers.toUtf8Bytes(JSON.stringify(receipt))
            ]);
            return ethers.keccak256(combined);
        });

        // Build merkle tree using Starknet Pedersen hash
        const tree = new MerkleTree(leaves);
        const proof = tree.getProof(txIndex);
        const root = tree.getRoot();

        console.log(`   Root: ${root}`);
        console.log(`   Proof length: ${proof.length} siblings`);

        return {
            root,
            siblings: proof
        };
    }

    async getContinuityChain(blockNumber) {
        console.log(`🔗 Fetching continuity chain for block ${blockNumber}...`);

        // In production, fetch from attestation cache/DB
        // For this example, we'll use mock data

        const startBlock = blockNumber - 10;
        const endBlock = blockNumber;

        const blockNumbers = [];
        const digests = [];

        for (let i = startBlock; i <= endBlock; i++) {
            blockNumbers.push(i);
            // In production, fetch actual attestation digests
            digests.push(ethers.randomBytes(32));
        }

        return {
            blockNumbers,
            digests
        };
    }

    encodeTxData(tx, receipt) {
        // Encode transaction and receipt data for verification
        return ethers.AbiCoder.defaultAbiCoder().encode(
            ['tuple(address from, address to, uint256 value, bytes data, uint256 gasUsed, uint256 status)'],
            [{
                from: tx.from,
                to: tx.to,
                value: tx.value,
                data: tx.data,
                gasUsed: receipt.gasUsed,
                status: receipt.status
            }]
        );
    }

    async submitProof(blockNumber, txIndex, txData, merkleProof, continuityChain) {
        console.log(`📤 Submitting proof to Creditcoin3...`);

        const query = {
            chainId: 1, // Ethereum
            blockNumber: blockNumber,
            txIndex: txIndex
        };

        const proof = {
            root: merkleProof.root,
            siblings: merkleProof.siblings
        };

        const continuity = {
            blockNumbers: continuityChain.blockNumbers,
            digests: continuityChain.digests
        };

        // Submit transaction
        const tx = await this.creditContract.verifyRepayment(
            query,
            txData,
            proof,
            continuity,
            {
                gasLimit: 3000000 // Adjust based on actual gas usage
            }
        );

        console.log(`   Transaction hash: ${tx.hash}`);

        // Wait for confirmation
        const receipt = await tx.wait();
        console.log(`   Confirmed in block: ${receipt.blockNumber}`);

        return receipt;
    }
}

// Start the relayer
const relayer = new RelayerService();
relayer.start().catch(console.error);
```

### Example 2: Smart Contract on Creditcoin3

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title Native Query Verifier Precompile Interface
/// @notice Interface for the native precompile at address 0x0BeA
interface INativeQueryVerifier {
    struct Query {
        uint64 chainId;
        uint64 blockNumber;
        uint64 txIndex;
    }

    struct MerkleProof {
        bytes32 root;
        bytes32[] siblings;
    }

    struct ContinuityChain {
        uint64[] blockNumbers;
        bytes32[] digests;
    }

    function verifyQuery(
        Query calldata query,
        bytes calldata txData,
        MerkleProof calldata proof,
        ContinuityChain calldata continuity
    ) external view returns (bool valid, bytes memory resultSegments);
}

/// @title Credit Rating System
/// @notice Automatically updates credit scores based on verified loan repayments
contract CreditRatingSystem {
    // Precompile at address 0x0BeA (example)
    INativeQueryVerifier constant VERIFIER = INativeQueryVerifier(address(0x0BeA));

    // Only trusted relayer can submit proofs
    address public relayer;

    // Credit scores
    mapping(address => uint256) public creditScores;

    // Prevent replay attacks
    mapping(bytes32 => bool) public processedProofs;

    event CreditScoreUpdated(address indexed borrower, uint256 amount, uint256 newScore);

    constructor(address _relayer) {
        relayer = _relayer;
    }

    /// @notice Verify a loan repayment and update credit score
    /// @param query The query specifying the source chain transaction
    /// @param txData The transaction data to verify
    /// @param proof The merkle proof for the transaction
    /// @param continuity The continuity chain for finality
    function verifyRepayment(
        INativeQueryVerifier.Query calldata query,
        bytes calldata txData,
        INativeQueryVerifier.MerkleProof calldata proof,
        INativeQueryVerifier.ContinuityChain calldata continuity
    ) external {
        // Only relayer can submit
        require(msg.sender == relayer, "Only relayer can submit");

        // Prevent replay attacks
        bytes32 proofHash = keccak256(abi.encode(query, txData));
        require(!processedProofs[proofHash], "Proof already processed");
        processedProofs[proofHash] = true;

        // Verify using native precompile
        (bool valid, bytes memory resultSegments) = VERIFIER.verifyQuery(
            query,
            txData,
            proof,
            continuity
        );

        require(valid, "Invalid proof");

        // Decode the verified data
        // Format depends on how you encoded it in the relayer
        (address borrower, uint256 amount) = abi.decode(
            resultSegments,
            (address, uint256)
        );

        // Update credit score
        uint256 bonus = calculateBonus(amount);
        creditScores[borrower] += bonus;

        emit CreditScoreUpdated(borrower, amount, creditScores[borrower]);
    }

    /// @notice Calculate credit score bonus based on repayment amount
    function calculateBonus(uint256 amount) internal pure returns (uint256) {
        // Simple example: 1 point per 100 wei
        return amount / 100;
    }

    /// @notice Update relayer address (only owner)
    function setRelayer(address _newRelayer) external {
        // Add access control (e.g., Ownable)
        relayer = _newRelayer;
    }
}
```

### Example 3: Merkle Tree Implementation

```javascript
// merkle.js
const { ethers } = require('ethers');

class MerkleTree {
    constructor(leaves) {
        this.leaves = leaves;
        this.layers = this.buildTree(leaves);
    }

    buildTree(leaves) {
        if (leaves.length === 0) {
            throw new Error('Cannot build tree with no leaves');
        }

        const layers = [leaves];

        while (layers[layers.length - 1].length > 1) {
            const currentLayer = layers[layers.length - 1];
            const nextLayer = [];

            for (let i = 0; i < currentLayer.length; i += 2) {
                if (i + 1 < currentLayer.length) {
                    // Hash pair
                    nextLayer.push(this.hashPair(currentLayer[i], currentLayer[i + 1]));
                } else {
                    // Odd number of nodes, carry forward
                    nextLayer.push(currentLayer[i]);
                }
            }

            layers.push(nextLayer);
        }

        return layers;
    }

    hashPair(left, right) {
        // Use Starknet Pedersen hash for compatibility
        // For this example, we'll use keccak256
        const concatenated = ethers.concat([left, right]);
        return ethers.keccak256(concatenated);
    }

    getRoot() {
        return this.layers[this.layers.length - 1][0];
    }

    getProof(index) {
        const proof = [];

        for (let i = 0; i < this.layers.length - 1; i++) {
            const layer = this.layers[i];
            const pairIndex = index % 2 === 0 ? index + 1 : index - 1;

            if (pairIndex < layer.length) {
                proof.push(layer[pairIndex]);
            }

            index = Math.floor(index / 2);
        }

        return proof;
    }

    verify(leaf, index, proof, root) {
        let hash = leaf;

        for (const sibling of proof) {
            if (index % 2 === 0) {
                hash = this.hashPair(hash, sibling);
            } else {
                hash = this.hashPair(sibling, hash);
            }
            index = Math.floor(index / 2);
        }

        return hash === root;
    }
}

module.exports = { MerkleTree };
```

---

## Deployment Guide

### Step 1: Deploy Smart Contract

```bash
# Deploy to Creditcoin3
npx hardhat run scripts/deploy.js --network creditcoin3
```

**deploy.js:**
```javascript
const { ethers } = require("hardhat");

async function main() {
    const [deployer] = await ethers.getSigners();
    console.log("Deploying with:", deployer.address);

    const relayerAddress = process.env.RELAYER_ADDRESS;

    const CreditRating = await ethers.getContractFactory("CreditRatingSystem");
    const contract = await CreditRating.deploy(relayerAddress);

    await contract.waitForDeployment();
    console.log("Contract deployed to:", await contract.getAddress());
}

main().catch(console.error);
```

### Step 2: Set Up Environment

Create `.env` file:
```bash
# Ethereum RPC
ETH_RPC_URL=https://mainnet.infura.io/v3/YOUR_KEY

# Creditcoin3 RPC
CC3_RPC_URL=https://rpc.creditcoin.network

# Relayer private key
RELAYER_PRIVATE_KEY=0x...

# Contract addresses
LOAN_CONTRACT_ADDRESS=0x...
CREDIT_CONTRACT_ADDRESS=0x...
```

### Step 3: Run Relayer

```bash
# Development
node relayer.js

# Production (with PM2)
pm2 start relayer.js --name creditcoin-relayer
pm2 save
pm2 startup
```

### Step 4: Monitor

```bash
# View logs
pm2 logs creditcoin-relayer

# Monitor performance
pm2 monit
```

---

## FAQ

### Q1: Can I do this entirely on-chain without a relayer?

**A: No.** Blockchains cannot make HTTP requests to other blockchains. You must have an off-chain component that fetches block data.

### Q2: What if my relayer goes offline?

**A: The system stops working.** For production, consider:
- Running multiple relayers (high availability)
- Using a decentralized oracle network
- Implementing automatic failover

### Q3: How much does the relayer cost to run?

**A:**
- **Infrastructure**: $50-200/month (VPS + monitoring)
- **Gas costs**: ~1.5-2.5M gas per proof × gas price × number of events
- **Total**: Depends on event frequency

### Q4: Can users submit proofs themselves?

**A: Yes,** but it's poor UX. Users would need to:
1. Run a proof generation tool
2. Pay gas costs
3. Understand the technical process

### Q5: Is the relayer trusted?

**A: Yes, in the basic implementation.** The relayer can:
- Choose which events to relay (censorship)
- Go offline (availability)

But the relayer **cannot:**
- Forge invalid proofs (precompile/STARK verifies)
- Modify data (cryptographically protected)

For trustless operation, use STARK proofs or decentralized oracles.

### Q6: Native precompile vs STARK - which should I use?

**A: Depends on your needs:**

| Requirement | Use |
|-------------|-----|
| Fast response (3-5 sec) | Native precompile |
| Trustless verification | STARK |
| Low cost | Native precompile |
| Cross-chain verification | STARK |
| Creditcoin-only | Native precompile |

### Q7: How do I handle chain reorganizations?

**A:** Wait for sufficient confirmations:
```javascript
const MIN_CONFIRMATIONS = 12; // ~3 minutes on Ethereum

loanContract.on('LoanRepaid', async (event) => {
    // Wait for confirmations
    const currentBlock = await ethProvider.getBlockNumber();
    const confirmations = currentBlock - event.blockNumber;

    if (confirmations < MIN_CONFIRMATIONS) {
        console.log(`Waiting for confirmations: ${confirmations}/${MIN_CONFIRMATIONS}`);
        return;
    }

    // Now safe to process
    await handleRepayment(event);
});
```

### Q8: How do I scale to high event volumes?

**A:**
1. **Queue system**: Use Redis/RabbitMQ to queue events
2. **Worker pool**: Run multiple worker processes
3. **Batch proofs**: Group multiple events into single proof (if supported)
4. **Database**: Store processed events to prevent re-processing

### Q9: What about privacy?

**A:** All data is publicly visible:
- Source chain transaction is public
- Merkle proof reveals transaction position
- Verified data is on-chain

For privacy, use:
- Zero-knowledge proofs (ZK-SNARKs)
- Private attestations
- Encrypted data segments

### Q10: Where can I get help?

**A:**
- [Creditcoin3 Documentation](https://docs.creditcoin.org)
- [GitHub Issues](https://github.com/gluwa/creditcoin3-next/issues)
- [Discord Community](https://discord.gg/creditcoin)

---

## Further Reading

- [PROOF_GENERATION_FLOW.md](./PROOF_GENERATION_FLOW.md) - Detailed flow diagrams
- [BLOCK_DATA_FLOW_DIAGRAMS.md](./BLOCK_DATA_FLOW_DIAGRAMS.md) - Visual data flow
- [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) - Performance comparison
- [WHAT_IS_BEING_PROVEN.md](./WHAT_IS_BEING_PROVEN.md) - Security properties
