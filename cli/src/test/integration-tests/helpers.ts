import { ApiPromise } from '@polkadot/api';
import { WasmPrivateKey } from 'bls-signatures-bindings';
import { BN, mnemonicGenerate } from '../../lib';
import { initKeyringPair, CallerKeyring } from '../../lib/account/keyring';
import { signSendAndWatchCcKeyring, TxStatus } from '../../lib/tx';
import { commandSync } from 'execa';
import { parseAmount } from '../../commands/options';
import { KeyringPair } from '../../lib';
import { substrateAddressToEvmAddress, evmAddressToSubstrateAddress } from '../../lib/evm/address';
import { HDNodeWallet } from 'ethers';
import { setStakingConfig } from '../../lib/staking/bond';
import { sleep } from '../utils';

export const ALICE_NODE_URL = 'ws://127.0.0.1:9944';
export const BOB_NODE_URL = 'ws://127.0.0.1:9955';
export const CLI_PATH = 'dist/cli.js';

export function fundFromSudo(api: ApiPromise, address: string, amount: BN) {
    const call = api.tx.balances.forceSetBalance(address, amount.toString());
    const tx = api.tx.sudo.sudo(call);
    const sudoKeyring: CallerKeyring = { type: 'caller', pair: initAliceKeyring() };
    return signSendAndWatchCcKeyring(tx, api, sudoKeyring);
}

export function fundAddressesFromSudo(api: ApiPromise, addresses: string[], amount: BN) {
    const txs = addresses.map((address) => {
        const fundTx = api.tx.balances.forceSetBalance(address, amount.toString());
        return api.tx.sudo.sudo(fundTx);
    });
    return api.tx.utility.batchAll(txs);
}

export async function waitEras(eras: number, api: ApiPromise) {
    let eraInfo = await api.derive.session.info();
    let currentEra = eraInfo.currentEra.toNumber();
    const targetEra = currentEra + eras;
    const blockTime = api.consts.babe.expectedBlockTime.toNumber();
    while (currentEra < targetEra) {
        console.log(`Waiting for era ${targetEra}, currently at ${currentEra}`);
        await sleep(blockTime);
        eraInfo = await api.derive.session.info();
        currentEra = eraInfo.currentEra.toNumber();
    }
}

export async function forceNewEra(api: ApiPromise) {
    const tx = api.tx.staking.forceNewEraAlways();
    const sudoTx = api.tx.sudo.sudo(tx);
    const sudoKeyring: CallerKeyring = { type: 'caller', pair: initAliceKeyring() };
    await signSendAndWatchCcKeyring(sudoTx, api, sudoKeyring);
}

export function randomTestAccount(secret = '') {
    if (secret === '') {
        secret = mnemonicGenerate();
    }
    const keyring = initKeyringPair(secret);
    const address = keyring.address;
    const evmAddress = substrateAddressToEvmAddress(address);
    // EVM keypair for precompile calls — derive via ethers.js HD wallet (BIP44 Ethereum path)
    const ethWallet = HDNodeWallet.fromPhrase(secret);
    const evmPrivateKey = ethWallet.privateKey;
    const ethEvmAddress = ethWallet.address;
    // The stash AccountId used by the attestor-stash precompile (HashedAddressMapping)
    const evmStashAddress = evmAddressToSubstrateAddress(ethEvmAddress);
    return { secret, keyring, address, evmAddress, evmPrivateKey, ethEvmAddress, evmStashAddress };
}

export function initAliceKeyring() {
    return initKeyringPair('//Alice');
}

export async function randomFundedAccount(api: ApiPromise, sudoSigner: KeyringPair, amount: BN = parseAmount('1000')) {
    const account = randomTestAccount();
    const fundTx = fundAddressesFromSudo(api, [account.address], amount);
    const sudoKeyring: CallerKeyring = { type: 'caller', pair: sudoSigner };
    await signSendAndWatchCcKeyring(fundTx, api, sudoKeyring);
    return account;
}

export async function increaseValidatorCount(api: ApiPromise, sudoSigner: KeyringPair, additional = 3) {
    const oldCount = (await api.query.staking.validatorCount()).toNumber();

    const sudoKeyring: CallerKeyring = { type: 'caller', pair: sudoSigner };
    await signSendAndWatchCcKeyring(
        api.tx.sudo.sudo(api.tx.staking.increaseValidatorCount(additional)),
        api,
        sudoKeyring,
    );

    const newCount = (await api.query.staking.validatorCount()).toNumber();
    expect(newCount).toEqual(oldCount + additional);
}

export function CLIBuilder(env: any) {
    let extraArgs = '';
    if (env.CC_PROXY_SECRET) {
        // WARNING: proxy setup must be done outside of this function
        const delegate = initKeyringPair(env.CC_SECRET);
        extraArgs = `--proxy-for ${delegate.address} --url ${BOB_NODE_URL}`;
    }

    function CLICmd(cmd: string) {
        return commandSync(`node ${CLI_PATH} ${cmd} ${extraArgs}`, { env });
    }
    return CLICmd;
}

export async function setUpProxy(api: ApiPromise, nonProxiedCli: any, delegate: any, proxy: any, wrongProxy: any) {
    if (process.env.PROXY_ENABLED === 'yes') {
        // this value isn't always defined properly
        let proxyType = process.env.PROXY_TYPE;
        if (proxyType === undefined || proxyType === '') {
            proxyType = 'All';
        }

        // eslint-disable-next-line @typescript-eslint/restrict-template-expressions
        const result = nonProxiedCli(`proxy add --proxy ${proxy.address} --type ${proxyType}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain('Transaction included at block');

        if (process.env.PROXY_SECRET_VARIANT === 'no-funds') {
            // will cause the configured proxy account not to have enough funds to pay fees
            await fundFromSudo(api, proxy.address, new BN(0));
        } else if (process.env.PROXY_SECRET_VARIANT === 'not-a-proxy') {
            // will cause CLI calls to use a proxy secret for a funded account which ISN'T
            // configured as a proxy for the delegate address. WARNING: outside of this function
            // the variable `proxy` will have its original value so you need to use wrongProxy.address
            // when assrting against error messages
            proxy = wrongProxy;
        }

        // make sure that our CLI instance uses the proxy account
        return CLIBuilder({ CC_SECRET: delegate.secret, CC_PROXY_SECRET: proxy.secret });
    }

    // or keep using the regular non-proxy CLI instance
    return nonProxiedCli;
}

export function tearDownProxy(cli: any, proxy: any) {
    if (process.env.PROXY_ENABLED === 'yes') {
        const result = cli(`proxy remove --proxy ${proxy.address}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain('Transaction included at block');
    }
}

export async function setMinBondConfig(api: ApiPromise, value: BN | number | string) {
    const sudoKeyring: CallerKeyring = { type: 'caller', pair: initAliceKeyring() };
    await setStakingConfig(sudoKeyring, api, null, value, null, null, null, null, null);
}

// Transition a registered attestor into the Active set by submitting `attest()`
// directly and polling until the election moves it from Waiting -> Active.
// This replaces spawning the external `attestor` binary in integration tests.
//
// BLS material is derived exactly as `pallet-attestation` expects (see
// `pallets/attestation/src/tests.rs` and `attestor/attestor/src/lib.rs`):
// the UTF-8 bytes of the mnemonic are used as IKM for filecoin-style
// `bls-signatures` KeyGen (`PrivateKey::new`), and the proof-of-possession
// signs the compressed public key bytes.
type AttestorStatusName = 'Active' | 'Idle' | 'Waiting' | 'NotRegistered';

async function readAttestorStatus(api: ApiPromise, chainKey: number, address: string): Promise<AttestorStatusName> {
    const attestorOption: any = await api.query.attestation.attestors(chainKey, address);
    if (!attestorOption.isSome) {
        return 'NotRegistered';
    }
    const statusEnum: any = attestorOption.unwrap().status;
    if (statusEnum.isActive) return 'Active';
    if (statusEnum.isIdle) return 'Idle';
    if (statusEnum.isWaiting) return 'Waiting';
    throw new Error(
        `activateAttestor: unknown AttestorStatus variant: ${statusEnum.toString()} (type=${statusEnum.type})`,
    );
}

export async function activateAttestor(
    api: ApiPromise,
    attestor: { secret: string; keyring: KeyringPair; address: string },
    chainKey: number,
    options: { pollIntervalMs?: number; timeoutMs?: number } = {},
): Promise<void> {
    const pollIntervalMs = options.pollIntervalMs ?? 3_000;
    const timeoutMs = options.timeoutMs ?? 360_000;

    const initialStatus = await readAttestorStatus(api, chainKey, attestor.address);
    if (initialStatus === 'NotRegistered') {
        throw new Error(
            `activateAttestor: attestor ${attestor.address} is not registered on chain ${chainKey} (call register first).`,
        );
    }

    if (initialStatus !== 'Active') {
        const blsKey = WasmPrivateKey.generate(new TextEncoder().encode(attestor.secret));
        const blsPublicKey = blsKey.public_key().as_bytes();
        const proofOfPossession = blsKey.sign(blsPublicKey).as_bytes();

        console.log(
            `[activateAttestor] submitting attest() for ${attestor.address} on chain ${chainKey} (current status=${initialStatus})`,
        );
        const tx = api.tx.attestation.attest(chainKey, blsPublicKey, proofOfPossession);
        const signer: CallerKeyring = { type: 'caller', pair: attestor.keyring };
        const result = await signSendAndWatchCcKeyring(tx, api, signer);
        if (result && result.status === TxStatus.failed) {
            throw new Error(`activateAttestor: attest() failed for ${attestor.address}: ${result.info}`);
        }
    }

    const deadline = Date.now() + timeoutMs;
    let lastStatus: AttestorStatusName = 'Idle';
    while (Date.now() < deadline) {
        lastStatus = await readAttestorStatus(api, chainKey, attestor.address);
        if (lastStatus === 'Active') {
            console.log(`[activateAttestor] ${attestor.address} is Active on chain ${chainKey}`);
            return;
        }
        await sleep(pollIntervalMs);
    }

    const active: string[] = [];
    const vec: any = await api.query.attestation.activeAttestors(chainKey);
    for (const account of vec) {
        active.push(account.toString());
    }
    throw new Error(
        `activateAttestor: attestor ${attestor.address} did not become Active within ${timeoutMs}ms on chain ${chainKey} ` +
            `(last attestor.status=${lastStatus}, ActiveAttestors=${JSON.stringify(active)}).`,
    );
}
