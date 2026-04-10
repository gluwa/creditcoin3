import { decodeAddress } from '@polkadot/util-crypto';
import { WebSocketProvider, ethers, hexlify, zeroPadValue, ContractFactory, HDNodeWallet } from 'ethers';

import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';
import { evmAddressToSubstrateAddress } from '../../../lib/evm/address';
import { chain_Anvil2_Key } from '../pallets/supported-chains/consts';
import { forElapsedBlocks } from '../../utils';
import type { SubmittableExtrinsic } from '@polkadot/api/types';
import type { KeyringPair } from '../../../lib';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import tokenArtifact = require('../artifacts/MockAttestToken.json');
// eslint-disable-next-line @typescript-eslint/no-require-imports
import precompileAbi = require('../artifacts/attest_coin_precompile.json');

const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';

async function signSendInBlock(signer: KeyringPair, tx: SubmittableExtrinsic<'promise'>) {
    await new Promise<void>((resolve, reject) => {
        let settled = false;
        tx.signAndSend(signer, (r) => {
            if (settled || !r.status.isInBlock) {
                return;
            }
            settled = true;
            if (r.dispatchError) {
                reject(r.dispatchError);
            } else {
                resolve();
            }
        }).catch(reject);
    });
}

describe('Precompile: attest-coin rewards (accrued / claim)', (): void => {
    let api: ApiPromise;
    let provider: ethers.WebSocketProvider;
    let evmWallet: HDNodeWallet;
    let stashSs58: string;
    let alice: any;
    let root: any;
    let tokenAddress: string;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        evmWallet = ethers.Wallet.createRandom().connect(provider);
        stashSs58 = evmAddressToSubstrateAddress(evmWallet.address);

        await fundFromSudo(api, stashSs58, MICROUNITS_PER_CTC.mul(new BN(50_000)));

        let nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, stashSs58)
            .signAndSend(alice, { nonce });
        await forElapsedBlocks(api, { minBlocks: 1 });

        const factory = new ContractFactory(tokenArtifact.abi, tokenArtifact.bytecode, evmWallet);
        const deployed = await factory.deploy();
        await deployed.waitForDeployment();
        tokenAddress = await deployed.getAddress();

        const precompileAddr = ATTEST_COIN_PRECOMPILE;
        const token = new ethers.Contract(tokenAddress, tokenArtifact.abi, evmWallet);
        const setMinterTx = await token.setMinter(precompileAddr);
        await setMinterTx.wait();

        const setTok = (api.tx as any).sudo.sudo(
            (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddress),
        );
        await signSendInBlock(root, setTok);

        const force = (api.tx as any).sudo.sudo((api.tx as any).attestCoinRewards.forceSettle());
        await signSendInBlock(root, force);
    }, 180_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test('accrued(bytes32) returns non-zero after forceSettle', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, provider);
        const raw = decodeAddress(stashSs58);
        const b32 = zeroPadValue(hexlify(raw), 32);
        const pts = await precompile.accrued(b32);
        expect(pts > 0n).toBe(true);
    });

    test('claim mints MockAttestToken to the caller', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWallet);
        const raw = decodeAddress(stashSs58);
        const b32 = zeroPadValue(hexlify(raw), 32);
        const ptsBefore = await precompile.accrued(b32);
        const claimAmt = ptsBefore / 2n > 0n ? ptsBefore / 2n : ptsBefore;

        const token = new ethers.Contract(tokenAddress, tokenArtifact.abi, provider);
        const balBefore = await token.balanceOf(evmWallet.address);

        const tx = await precompile.claim(claimAmt, { gasLimit: 800_000n });
        await tx.wait();

        const balAfter = await token.balanceOf(evmWallet.address);
        expect(balAfter - balBefore).toEqual(claimAmt);
    }, 120_000);
});
