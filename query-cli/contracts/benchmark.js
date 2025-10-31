// benchmark.js - Script to deploy and run gas benchmarks for native query verification

const { ethers } = require("hardhat");

async function main() {
    console.log("=".repeat(80));
    console.log("Native Query Verification - Gas Benchmark");
    console.log("=".repeat(80));

    // Deploy the benchmark contract
    console.log("\n📦 Deploying GasBenchmark contract...");
    const GasBenchmark = await ethers.getContractFactory("GasBenchmark");
    const benchmark = await GasBenchmark.deploy();
    await benchmark.deployed();
    console.log(`✅ Contract deployed at: ${benchmark.address}`);

    // Get deployment transaction receipt for deployment gas cost
    const deployTx = benchmark.deployTransaction;
    const deployReceipt = await deployTx.wait();
    console.log(`   Deployment gas used: ${deployReceipt.gasUsed.toString()}`);

    console.log("\n" + "=".repeat(80));
    console.log("Running Benchmarks");
    console.log("=".repeat(80));

    // Test Case 1: Single transaction block (like your block #736)
    console.log("\n📊 Test Case 1: Single Transaction Block");
    console.log("   - Transaction size: 991 bytes");
    console.log("   - Merkle siblings: 0 (single tx)");
    console.log("   - Continuity blocks: 6");
    console.log("   - Data segments: 4");

    const singleTxResult = await runBenchmark(benchmark, {
        txSize: 991,
        merkleDepth: 0,
        continuityLength: 6,
        segments: [
            { offset: 479, size: 32 },
            { offset: 223, size: 32 },
            { offset: 255, size: 32 },
            { offset: 287, size: 32 }
        ]
    });

    // Test Case 2: Multi-transaction block
    console.log("\n📊 Test Case 2: Multi-Transaction Block");
    console.log("   - Transaction size: 991 bytes");
    console.log("   - Merkle siblings: 6 (64 txs, 3 levels)");
    console.log("   - Continuity blocks: 10");
    console.log("   - Data segments: 4");

    const multiTxResult = await runBenchmark(benchmark, {
        txSize: 991,
        merkleDepth: 3,
        continuityLength: 10,
        segments: [
            { offset: 479, size: 32 },
            { offset: 223, size: 32 },
            { offset: 255, size: 32 },
            { offset: 287, size: 32 }
        ]
    });

    // Test Case 3: Large transaction with many segments
    console.log("\n📊 Test Case 3: Large Transaction");
    console.log("   - Transaction size: 10,000 bytes");
    console.log("   - Merkle siblings: 8 (256 txs, 4 levels)");
    console.log("   - Continuity blocks: 20");
    console.log("   - Data segments: 10");

    const largeTxResult = await runBenchmark(benchmark, {
        txSize: 10000,
        merkleDepth: 4,
        continuityLength: 20,
        segments: Array(10).fill(null).map((_, i) => ({
            offset: i * 100,
            size: 32
        }))
    });

    // Test individual operations
    console.log("\n" + "=".repeat(80));
    console.log("Individual Operation Benchmarks");
    console.log("=".repeat(80));

    // Benchmark Pedersen hash simulation
    console.log("\n🔬 Hash Operations (simulated with keccak256):");
    const hashSizes = [32, 100, 500, 1000, 5000];
    for (const size of hashSizes) {
        const data = ethers.utils.randomBytes(size);
        const tx = await benchmark.benchmarkPedersenHash(data);
        const receipt = await tx.wait();
        const gasUsed = receipt.gasUsed.sub(21000); // Subtract base transaction cost
        console.log(`   ${size} bytes: ${gasUsed.toString()} gas`);
    }

    // Benchmark storage lookup
    console.log("\n🔬 Storage Lookup:");
    const key = ethers.utils.randomBytes(32);
    const storageTx = await benchmark.benchmarkStorageLookup(key);
    const storageReceipt = await storageTx.wait();
    console.log(`   Single lookup: ${storageReceipt.gasUsed.sub(21000).toString()} gas`);

    // Benchmark memory copy
    console.log("\n🔬 Memory Copy Operations:");
    const copySizes = [32, 100, 500, 1000];
    const testData = ethers.utils.randomBytes(2000);
    for (const size of copySizes) {
        const tx = await benchmark.benchmarkMemcpy(testData, 0, size);
        const receipt = await tx.wait();
        const gasUsed = receipt.gasUsed.sub(21000);
        console.log(`   ${size} bytes: ${gasUsed.toString()} gas`);
    }

    // Summary and recommendations
    console.log("\n" + "=".repeat(80));
    console.log("Summary & Recommendations");
    console.log("=".repeat(80));

    console.log("\n📈 Actual Gas Costs vs Current Constants:");
    console.log("   Current precompile constants:");
    console.log("   - GAS_BASE_VERIFY: 50,000");
    console.log("   - GAS_PER_TX_BYTE: 10");
    console.log("   - GAS_PER_SIBLING: 3,000");
    console.log("   - GAS_PER_CONTINUITY_BLOCK: 5,000");

    console.log("\n   Measured costs (approximate):");
    console.log(`   - Base verification: ${singleTxResult.base} gas`);
    console.log(`   - Per TX byte: ${Math.floor(singleTxResult.perByte)} gas`);
    console.log(`   - Per sibling: ${Math.floor(multiTxResult.perSibling)} gas`);
    console.log(`   - Per continuity block: ${Math.floor(singleTxResult.perBlock)} gas`);

    console.log("\n💡 Recommendations:");
    if (singleTxResult.perByte > 10) {
        console.log("   ⚠️  GAS_PER_TX_BYTE should be increased");
    }
    if (multiTxResult.perSibling > 3000) {
        console.log("   ⚠️  GAS_PER_SIBLING should be increased");
    }
    if (singleTxResult.perBlock > 5000) {
        console.log("   ⚠️  GAS_PER_CONTINUITY_BLOCK should be increased");
    }

    console.log("\n✅ Benchmark complete!");
}

async function runBenchmark(benchmark, params) {
    // Generate test data
    const txData = ethers.utils.randomBytes(params.txSize);

    // Build query
    const query = {
        chainId: 2,
        height: 736,
        index: 0,
        layoutSegments: params.segments
    };

    // Build Merkle proof
    const merkleProof = {
        root: ethers.utils.keccak256(txData),
        siblings: Array(params.merkleDepth * 2).fill(ethers.utils.randomBytes(32))
    };

    // Build continuity blocks
    const continuityBlocks = [];
    for (let i = 0; i < params.continuityLength; i++) {
        continuityBlocks.push({
            blockNumber: i + 731, // Start from block after attestation
            root: ethers.utils.randomBytes(32),
            prevDigest: i === 0 ? ethers.utils.randomBytes(32) :
                       ethers.utils.keccak256(ethers.utils.toUtf8Bytes(`block${i-1}`)),
            digest: ethers.utils.keccak256(ethers.utils.toUtf8Bytes(`block${i}`))
        });
    }

    // Run benchmark
    const tx = await benchmark.benchmarkVerification(
        query,
        txData,
        merkleProof,
        continuityBlocks
    );

    const receipt = await tx.wait();
    const totalGas = receipt.gasUsed.toNumber();

    // Parse events for detailed breakdown
    const events = receipt.events || [];
    const gasBreakdown = {};

    for (const event of events) {
        if (event.event === 'GasConsumed') {
            const [operation, gasUsed] = event.args;
            gasBreakdown[operation] = gasUsed.toNumber();
            console.log(`   - ${operation}: ${gasUsed.toString()} gas`);
        }
    }

    console.log(`   Total gas used: ${totalGas}`);

    // Calculate per-unit costs
    return {
        total: totalGas,
        base: totalGas - (params.txSize * 10), // Rough estimate
        perByte: gasBreakdown['MerkleVerification'] ?
                 gasBreakdown['MerkleVerification'] / params.txSize : 0,
        perSibling: params.merkleDepth > 0 ?
                    gasBreakdown['MerkleVerification'] / (params.merkleDepth * 2) : 0,
        perBlock: gasBreakdown['ContinuityVerification'] ?
                  gasBreakdown['ContinuityVerification'] / params.continuityLength : 0,
        breakdown: gasBreakdown
    };
}

// Run the benchmark
main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error(error);
        process.exit(1);
    });
