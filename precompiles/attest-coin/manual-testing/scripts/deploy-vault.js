'use strict';

// Step 4b — deploy the attestcoin treasury vault and grant the precompile its allowance.
//
// Reward claims are paid with `transferFrom(vault, attestor, amount)`: the precompile is the
// vault's approved *spender*, never the holder of reward funds. So a claim needs three things,
// and this script does the two EVM-side ones:
//
//   1. a deployed vault holding ATC            <- here (funded separately, step 5)
//   2. an allowance for the precompile         <- here (`--cap`, default unlimited)
//   3. `AttestCoinRewards.setRewardVault`      <- sudo, see the readme
//
// The deployer becomes both `owner` and `guardian`, so the same key can top up the cap and
// exercise `pauseRewardRedemptions()` while testing.
//
// Usage:
//   node scripts/deploy-vault.js              # unlimited allowance
//   node scripts/deploy-vault.js --cap 5000   # 5,000 ATC spending cap
//
// Caps are whole ATC; the script applies the token's 18 decimals.

const fs = require('fs');
const path = require('path');
const { ethers } = require('ethers');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
// Generated, not hand-maintained: `cli/scripts/sync-vault-artifact.sh` compiles it from the
// AttestCoinTreasuryVault.sol that ships in the @gluwa/asc-contracts npm package. The contract
// itself lives in the asc-contracts repo -- edit it there, not here.
const ARTIFACT_PATH = path.join(
    REPO_ROOT,
    'cli/src/test/blockchain-tests/artifacts/AttestCoinTreasuryVault.json',
);
const SYNC_HINT =
    'Regenerate it with:\n  cd cli && yarn sync:vault-artifact  (needs solc on PATH)';

/** Attest-coin precompile, `PrecompileAt<AddressU64<4053>>` — 4053 == 0xfd5. */
const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const DEPLOYER_PRIVATE_KEY = process.env.DEPLOYER_PRIVATE_KEY;
const TOKEN_ADDRESS = process.env.ATTESTCOIN_ERC20;

const USAGE = `usage: node scripts/deploy-vault.js [--cap <whole-ATC>]

  node scripts/deploy-vault.js
  node scripts/deploy-vault.js --cap 5000`;

/**
 * Set `key=value` in .env.
 *
 * Replaces **every** existing line for the key rather than the first, so a file that already
 * picked up duplicates (e.g. from a `>>` append) heals itself instead of growing a second
 * stale copy. Only a genuinely absent key is appended.
 */
function setEnvKey(contents, key, value) {
    const line = `${key}=${value}`;
    const existing = new RegExp(`^${key}=.*$`, 'gm');
    if (!existing.test(contents)) {
        return `${contents.replace(/\n*$/, '\n')}${line}\n`;
    }
    let first = true;
    return contents
        .replace(new RegExp(`^${key}=.*$\n?`, 'gm'), () => {
            if (first) {
                first = false;
                return `${line}\n`;
            }
            return '';
        })
        .replace(/\n*$/, '\n');
}

/** Record the deployed address back into .env so later steps can read it. */
function writeEnvAddress(address) {
    const contents = fs.readFileSync(ENV_PATH, 'utf8');
    fs.writeFileSync(ENV_PATH, setEnvKey(contents, 'ATTESTCOIN_VAULT', address));
}

function parseCap(argv) {
    const i = argv.indexOf('--cap');
    if (i === -1) {
        return ethers.MaxUint256;
    }
    const raw = argv[i + 1];
    if (!raw) {
        throw new Error(`--cap needs a value\n\n${USAGE}`);
    }
    const cap = ethers.parseUnits(raw, 18);
    if (cap <= 0n) {
        throw new Error(`cap must be positive, got "${raw}"`);
    }
    return cap;
}

async function main() {
    const cap = parseCap(process.argv.slice(2));

    if (!DEPLOYER_PRIVATE_KEY) {
        throw new Error('DEPLOYER_PRIVATE_KEY is not set — see .env');
    }
    if (!TOKEN_ADDRESS) {
        throw new Error('ATTESTCOIN_ERC20 is not set in .env — run step 3 (deploy-erc20.js) first');
    }

    let artifact;
    try {
        artifact = JSON.parse(fs.readFileSync(ARTIFACT_PATH, 'utf8'));
    } catch (error) {
        throw new Error(
            `cannot read the vault artifact at ${ARTIFACT_PATH}\n  ${error.message}\n\n${SYNC_HINT}`,
        );
    }
    if (!artifact.abi || !artifact.bytecode || artifact.bytecode === '0x') {
        throw new Error(`vault artifact at ${ARTIFACT_PATH} has no abi/bytecode\n\n${SYNC_HINT}`);
    }
    const provider = new ethers.JsonRpcProvider(RPC_URL);

    let network;
    try {
        network = await provider.getNetwork();
    } catch (error) {
        throw new Error(
            `cannot reach the Creditcoin EVM at ${RPC_URL} — is the node running?\n  ${error.message}`,
        );
    }

    if ((await provider.getCode(TOKEN_ADDRESS)) === '0x') {
        throw new Error(
            `no contract at ATTESTCOIN_ERC20=${TOKEN_ADDRESS} — did the chain restart since step 3?`,
        );
    }

    const deployer = new ethers.Wallet(DEPLOYER_PRIVATE_KEY, provider);
    const precompile = ethers.getAddress(ATTEST_COIN_PRECOMPILE);

    console.log(`RPC          ${RPC_URL} (evm chain id ${network.chainId})`);
    console.log(`token        ${TOKEN_ADDRESS}`);
    console.log(`deployer     ${deployer.address}  (vault owner + guardian)`);
    console.log(`spender      ${precompile}  (attest-coin precompile)`);

    const factory = new ethers.ContractFactory(artifact.abi, artifact.bytecode, deployer);
    // constructor(token, owner, spender, guardian)
    const vault = await factory.deploy(
        TOKEN_ADDRESS,
        deployer.address,
        precompile,
        deployer.address,
    );
    await vault.waitForDeployment();
    const address = await vault.getAddress();

    const receipt = await (await vault.setAllowance(cap)).wait();

    writeEnvAddress(address);

    const capLabel = cap === ethers.MaxUint256 ? 'unlimited' : `${ethers.formatUnits(cap, 18)} ATC`;
    console.log(`\nvault        ${address}`);
    console.log(`allowance    ${capLabel}  (tx ${receipt.hash})`);
    console.log(`\nSaved ATTESTCOIN_VAULT=${address} to ${ENV_PATH}`);
    console.log('\nNext: register it with the runtime (sudo), then fund it:');
    console.log('  polkadot.js -> Developer -> Sudo -> AttestCoinRewards -> setRewardVault');
    console.log(`    vault -> ${address}`);
    console.log('  node scripts/fund-erc20.js vault 10000');
}

main().catch((error) => {
    console.error(`\nFAILED: ${error.message}`);
    process.exit(1);
});
