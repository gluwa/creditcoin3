'use strict';

// Step 5b — mint attest-coin ERC-20 (ATC) to a target EVM account.
//
// Sudo cannot do this. ATC is an ordinary ERC-20 living on Creditcoin's EVM,
// and `MockAttestToken.mint` is restricted to the account that deployed it, so
// the mint has to be signed with DEPLOYER_PRIVATE_KEY — the same key step 3
// used. Only the CTC half of step 5 is a sudo operation.
//
// Usage:
//   node scripts/fund-erc20.js precompile 10000     # the precompile's ERC-20 treasury
//   node scripts/fund-erc20.js 0x<address> 100      # e.g. the attestor stash
//
// Amounts are whole ATC; the script applies the token's 18 decimals.

const fs = require('fs');
const path = require('path');
const { ethers } = require('ethers');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const ARTIFACT_PATH = path.join(
    REPO_ROOT,
    'cli/src/test/blockchain-tests/artifacts/MockAttestToken.json',
);

/** Attest-coin precompile, `PrecompileAt<AddressU64<4053>>` — 4053 == 0xfd5. */
const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const DEPLOYER_PRIVATE_KEY = process.env.DEPLOYER_PRIVATE_KEY;
const TOKEN_ADDRESS = process.env.ATTESTCOIN_ERC20;

const USAGE = `usage: node scripts/fund-erc20.js <precompile|0xADDRESS> <whole-ATC>

  node scripts/fund-erc20.js precompile 10000
  node scripts/fund-erc20.js 0x1234...cdef 100`;

/** Accept the `precompile` alias so nobody has to memorise 0x…0fd5. */
function resolveTarget(arg) {
    if (arg.toLowerCase() === 'precompile') {
        return ethers.getAddress(ATTEST_COIN_PRECOMPILE);
    }
    if (!ethers.isAddress(arg)) {
        throw new Error(`"${arg}" is not an EVM address (or the "precompile" alias)\n\n${USAGE}`);
    }
    return ethers.getAddress(arg);
}

async function main() {
    const [targetArg, amountArg] = process.argv.slice(2);
    if (!targetArg || !amountArg) {
        throw new Error(USAGE);
    }
    if (!DEPLOYER_PRIVATE_KEY) {
        throw new Error('DEPLOYER_PRIVATE_KEY is not set — see .env');
    }
    if (!TOKEN_ADDRESS) {
        throw new Error('ATTESTCOIN_ERC20 is not set in .env — run step 3 (deploy-erc20.js) first');
    }

    const target = resolveTarget(targetArg);
    const amount = ethers.parseUnits(amountArg, 18);
    if (amount <= 0n) {
        throw new Error(`amount must be positive, got "${amountArg}"`);
    }

    const artifact = JSON.parse(fs.readFileSync(ARTIFACT_PATH, 'utf8'));
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

    const minter = new ethers.Wallet(DEPLOYER_PRIVATE_KEY, provider);
    const token = new ethers.Contract(TOKEN_ADDRESS, artifact.abi, minter);

    const onChainMinter = await token.minter();
    if (onChainMinter.toLowerCase() !== minter.address.toLowerCase()) {
        throw new Error(
            `${minter.address} is not the token's minter (${onChainMinter}) — only the deployer can mint`,
        );
    }

    const before = await token.balanceOf(target);

    console.log(`RPC          ${RPC_URL} (evm chain id ${network.chainId})`);
    console.log(`token        ${TOKEN_ADDRESS}`);
    console.log(`minter       ${minter.address}`);
    console.log(
        `target       ${target}${target === ethers.getAddress(ATTEST_COIN_PRECOMPILE) ? '  (attest-coin precompile)' : ''}`,
    );
    console.log(`balance      ${ethers.formatUnits(before, 18)} ATC`);

    const receipt = await (await token.mint(target, amount)).wait();
    const after = await token.balanceOf(target);

    console.log(`\nminted       ${ethers.formatUnits(amount, 18)} ATC  (tx ${receipt.hash})`);
    console.log(`balance      ${ethers.formatUnits(after, 18)} ATC`);
}

main().catch((error) => {
    console.error(`\nFAILED: ${error.message}`);
    process.exit(1);
});
