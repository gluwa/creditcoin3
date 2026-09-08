'use strict';

// Step 3 — deploy the attest-coin ERC-20 on the Creditcoin EVM.
//
// The token has to live on Creditcoin's own embedded EVM rather than on anvil:
// the attest-coin precompile reaches it with direct `transferFrom` / `transfer`
// / `balanceOf` subcalls, so the two must share a chain. Anvil is only the
// source chain the attestors attest to.
//
// The contract is `MockAttestToken` — the same mintable ERC-20 subset the
// blockchain integration tests configure as attest coin. Its prebuilt artifact
// is reused from the CLI test tree, so no solc is needed here.
//
// The deployer becomes the token's `minter` and nobody else can mint, so step 5
// has to fund the precompile treasury and the stash with this same key.

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

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const DEPLOYER_PRIVATE_KEY = process.env.DEPLOYER_PRIVATE_KEY;

/** Record the deployed address back into .env so later steps can read it. */
function writeEnvAddress(address) {
    const line = `ATTESTCOIN_ERC20=${address}`;
    const contents = fs.readFileSync(ENV_PATH, 'utf8');
    const updated = /^ATTESTCOIN_ERC20=.*$/m.test(contents)
        ? contents.replace(/^ATTESTCOIN_ERC20=.*$/m, line)
        : `${contents.replace(/\n*$/, '\n')}${line}\n`;
    fs.writeFileSync(ENV_PATH, updated);
}

async function main() {
    if (!DEPLOYER_PRIVATE_KEY) {
        throw new Error('DEPLOYER_PRIVATE_KEY is not set — see .env');
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

    const deployer = new ethers.Wallet(DEPLOYER_PRIVATE_KEY, provider);
    const balance = await provider.getBalance(deployer.address);

    console.log(`RPC          ${RPC_URL} (evm chain id ${network.chainId})`);
    console.log(`deployer     ${deployer.address}`);
    console.log(`balance      ${ethers.formatEther(balance)} CTC`);

    if (balance === 0n) {
        throw new Error(
            `deployer ${deployer.address} has no CTC — it must be an account endowed by the --dev chainspec`,
        );
    }

    const factory = new ethers.ContractFactory(artifact.abi, artifact.bytecode, deployer);
    const contract = await factory.deploy();
    console.log(`\ndeploying    ${contract.deploymentTransaction().hash}`);

    await contract.waitForDeployment();
    const address = await contract.getAddress();
    const minter = await contract.minter();

    writeEnvAddress(address);

    console.log(`deployed     ${address}`);
    console.log(`minter       ${minter}`);
    console.log(`\nSaved ATTESTCOIN_ERC20=${address} to ${ENV_PATH}`);
    console.log(`
Next (step 4) — tell the runtime this ERC-20 backs pallet-assets id 1:
  Polkadot.js -> Developer -> Sudo -> attestCoinRewards.setAttestCoinToken
    token: ${address}`);
}

main().catch((error) => {
    console.error(`\nFAILED: ${error.message}`);
    process.exit(1);
});
