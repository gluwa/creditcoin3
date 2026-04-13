import { Command, OptionValues } from 'commander';
import { decodeAddress } from '@polkadot/util-crypto';
import { Contract, JsonRpcProvider, hexlify, zeroPadValue } from 'ethers';
import { newApi } from '../../lib';
import { substrateAddressOption } from '../options';

const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';

const PRECOMPILE_ABI = [
    {
        inputs: [{ internalType: 'bytes32', name: 'stash', type: 'bytes32' }],
        name: 'accrued',
        outputs: [{ internalType: 'uint256', name: '', type: 'uint256' }],
        stateMutability: 'view',
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

/** Next EVM-claim nonce for a stash (see precompile `claim` + sr25519 signing). */
export function makeAttestCoinClaimNonceCommand() {
    const cmd = new Command('claim-nonce');
    cmd.description('Show on-chain claim nonce for a Substrate stash (paired with sr25519-signed precompile claim)');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(attestCoinClaimNonceAction);
    return cmd;
}

async function attestCoinClaimNonceAction(options: OptionValues) {
    const url = options.url as string;
    const stash = options.substrateAddress as string;
    const { api } = await newApi(url);
    try {
        const pallet = (api.query as any).attestCoinRewards;
        if (!pallet?.claimNonce) {
            console.error('Chain metadata has no pallet attestCoinRewards.claimNonce.');
            process.exit(1);
        }
        const raw = await pallet.claimNonce(stash);
        console.log((raw as { toString: () => string }).toString());
    } finally {
        await api.disconnect();
    }
}
