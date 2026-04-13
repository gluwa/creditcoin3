import { decodeAddress } from '@polkadot/util-crypto';
import { WebSocketProvider, ethers, hexlify, zeroPadValue, ContractFactory } from 'ethers';

import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';
import { chain_Anvil2_Key } from '../pallets/supported-chains/consts';
import { forElapsedBlocks } from '../../utils';
import type { SubmittableExtrinsic } from '@polkadot/api/types';
import type { KeyringPair } from '../../../lib';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import tokenArtifact = require('../artifacts/MockAttestToken.json');
// eslint-disable-next-line @typescript-eslint/no-require-imports
import precompileAbi = require('../artifacts/attest_coin_precompile.json');

const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';

function toLeU64(n: bigint): Buffer {
    let v = n;
    const b = Buffer.alloc(8);
    for (let i = 0; i < 8; i++) {
        b[i] = Number(v & 0xffn);
        v >>= 8n;
    }
    return b;
}

function toLeU128(n: bigint): Buffer {
    let v = n;
    const b = Buffer.alloc(16);
    for (let i = 0; i < 16; i++) {
        b[i] = Number(v & 0xffn);
        v >>= 8n;
    }
    return b;
}

/** Must match `pallet_attest_coin_rewards::Pallet::claim_signing_message`. */
function buildClaimSigningMessage(
    stash32: Uint8Array,
    nonce: bigint,
    chainKey: bigint,
    amount: bigint,
    evmRecipient20: Uint8Array,
): Uint8Array {
    const prefix = Buffer.from('AttestCoin:claim:v1:', 'utf8');
    return new Uint8Array(
        Buffer.concat([
            prefix,
            Buffer.from(stash32),
            toLeU64(nonce),
            toLeU64(chainKey),
            toLeU128(amount),
            Buffer.from(evmRecipient20),
        ]),
    );
}

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
    /** Alith — must match //Alice per dev key mapping. */
    let evmWallet: ethers.Wallet;
    let alice: KeyringPair;
    let root: KeyringPair;
    let tokenAddress: string;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        const pk = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        evmWallet = new ethers.Wallet(pk, provider);

        await fundFromSudo(api, alice.address, MICROUNITS_PER_CTC.mul(new BN(50_000)));

        let nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, alice.address)
            .signAndSend(alice, { nonce });
        await forElapsedBlocks(api, { minBlocks: 1 });

        const factory = new ContractFactory(tokenArtifact.abi, tokenArtifact.bytecode, evmWallet);
        const deployed = await factory.deploy();
        await deployed.waitForDeployment();
        tokenAddress = await deployed.getAddress();

        const token = new ethers.Contract(tokenAddress, tokenArtifact.abi, evmWallet);
        const mintTx = await token.mint(ATTEST_COIN_PRECOMPILE, ethers.parseEther('1000000'));
        await mintTx.wait();

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
        const raw = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(raw), 32);
        const pts = await precompile.accrued(b32);
        expect(pts > 0n).toBe(true);
    });

    test('claim transfers MockAttestToken from precompile treasury with sr25519', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWallet);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);

        const ptsBefore = await precompile.accrued(b32);
        const claimAmt = ptsBefore / 2n > 0n ? ptsBefore / 2n : ptsBefore;

        const token = new ethers.Contract(tokenAddress, tokenArtifact.abi, provider);
        const balBefore = await token.balanceOf(evmWallet.address);

        const evmRecipient = ethers.getBytes(evmWallet.address);
        const msg = buildClaimSigningMessage(stashU8, 0n, BigInt(chain_Anvil2_Key), claimAmt, evmRecipient);
        const sig = alice.sign(msg);
        if (sig.length !== 64) {
            throw new Error(`expected 64-byte sr25519 signature, got ${sig.length}`);
        }
        const sigHi = ethers.hexlify(sig.subarray(0, 32));
        const sigLo = ethers.hexlify(sig.subarray(32, 64));

        const tx = await precompile.claim(
            b32,
            0n,
            BigInt(chain_Anvil2_Key),
            claimAmt,
            evmWallet.address,
            sigHi,
            sigLo,
            { gasLimit: 1_000_000n },
        );
        await tx.wait();

        const balAfter = await token.balanceOf(evmWallet.address);
        expect(balAfter - balBefore).toEqual(claimAmt);
    }, 120_000);
});
