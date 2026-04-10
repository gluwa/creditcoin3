import { Command, OptionValues } from 'commander';
import { decodeAddress } from '@polkadot/util-crypto';
import { Contract, JsonRpcProvider, Wallet, hexlify, zeroPadValue } from 'ethers';
import { newApi } from '../../lib';
import { substrateAddressOption } from '../options';

const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';

const PRECOMPILE_ABI = [
    {
        inputs: [{ internalType: 'bytes32', name: 'stash', type: 'bytes32' }],
        name: 'accrued',
        outputs: [{ internalType: 'uint256', name: '', type: 'uint256' }],
        stateMutability: 'view',
        type: 'function',
    },
    {
        inputs: [{ internalType: 'uint256', name: 'amount', type: 'uint256' }],
        name: 'claim',
        outputs: [],
        stateMutability: 'nonpayable',
        type: 'function',
    },
] as const;

function httpRpcFromWs(ws: string): string {
    if (ws.startsWith('ws://')) {
        return 'http://' + ws.slice('ws://'.length);
    }
    if (ws.startsWith('wss://')) {
        return 'https://' + ws.slice('wss://'.length);
    }
    return ws;
}

/** View accrued reward points for a Substrate stash (runtime storage). */
export function makeAttestCoinAccruedCommand() {
    const cmd = new Command('accrued');
    cmd.description('Show accrued attest-coin reward points for a Substrate stash (runtime storage)');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(attestCoinAccruedAction);
    return cmd;
}

async function attestCoinAccruedAction(options: OptionValues) {
    const url = options.url as string;
    const stash = options.substrateAddress as string;
    const { api } = await newApi(url);
    try {
        const pallet = (api.query as any).attestCoinRewards;
        if (!pallet?.accrued) {
            console.error(
                'Chain metadata has no pallet attestCoinRewards.accrued. Connect to a node that includes the pallet.',
            );
            process.exit(1);
        }
        const raw = await pallet.accrued(stash);
        const pts = (raw as { toString: () => string }).toString();
        console.log(pts);
    } finally {
        await api.disconnect();
    }
}

/** Read accrued points via precompile `accrued(bytes32)`. */
export function makeAttestCoinAccruedEvmCommand() {
    const cmd = new Command('accrued-evm');
    cmd.description('Read accrued points via EVM precompile accrued(bytes32) for a Substrate stash');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(attestCoinAccruedEvmAction);
    return cmd;
}

async function attestCoinAccruedEvmAction(options: OptionValues) {
    const url = options.url as string;
    const stash = options.substrateAddress as string;
    const provider = new JsonRpcProvider(httpRpcFromWs(url));
    const contract = new Contract(ATTEST_COIN_PRECOMPILE, PRECOMPILE_ABI, provider);
    const raw = decodeAddress(stash);
    const b32 = zeroPadValue(hexlify(raw), 32);
    const v = await contract.accrued(b32);
    console.log(v.toString());
}

/** Claim reward points via EVM precompile (mints configured ERC-20 to the caller). */
export function makeAttestCoinClaimCommand() {
    const cmd = new Command('claim');
    cmd.description('Claim attest-coin rewards via EVM precompile (set CC_EVM_PRIVATE_KEY to the claimer EVM key)');
    cmd.requiredOption(
        '-a, --amount <amount>',
        'Amount to claim in wei (must not exceed accrued balance)',
    );
    cmd.action(attestCoinClaimAction);
    return cmd;
}

async function attestCoinClaimAction(options: OptionValues) {
    const url = options.url as string;
    const amountStr = options.amount as string;
    if (!/^\d+$/.test(amountStr)) {
        console.error('amount must be a positive integer string (wei)');
        process.exit(1);
    }

    const pk = process.env.CC_EVM_PRIVATE_KEY;
    if (!pk || !isValidHexKey(pk)) {
        console.error('Set CC_EVM_PRIVATE_KEY to a 0x-prefixed 32-byte EVM private key for the claiming account.');
        process.exit(1);
    }

    const provider = new JsonRpcProvider(httpRpcFromWs(url));
    const wallet = new Wallet(pk, provider);
    const contract = new Contract(ATTEST_COIN_PRECOMPILE, PRECOMPILE_ABI, wallet);

    const tx = await contract.claim(amountStr, { gasLimit: 500_000n });
    const receipt = await tx.wait();
    console.log(receipt?.hash ?? tx.hash);
}

function isValidHexKey(k: string) {
    return /^0x[0-9a-fA-F]{64}$/.test(k);
}
