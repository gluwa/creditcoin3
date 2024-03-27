import { ApiPromise } from '@polkadot/api';
import { BN, mnemonicGenerate, newApi } from '../../lib';
import { initKeyringPair } from '../../lib/account/keyring';
import { signSendAndWatch } from '../../lib/tx';
import { commandSync } from 'execa';
import { parseAmount } from '../../commands/options';
import { KeyringPair } from '../../lib';
import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { setStakingConfig } from '../../lib/staking/bond';
import { sleep } from '../utils';

export const ALICE_NODE_URL = 'ws://127.0.0.1:9944';
export const BOB_NODE_URL = 'ws://127.0.0.1:9955';
export const CLI_PATH = 'dist/cli.js';

export async function fundFromSudo(address: string, amount: BN, url = ALICE_NODE_URL) {
    const { api } = await newApi(url);
    const call = api.tx.balances.forceSetBalance(address, amount.toString());
    const tx = api.tx.sudo.sudo(call);
    return signSendAndWatch(tx, api, initAliceKeyring());
}

export async function fundAddressesFromSudo(addresses: string[], amount: BN, url = ALICE_NODE_URL) {
    const { api } = await newApi(url);
    const txs = addresses.map((address) => {
        const fundTx = api.tx.balances.forceSetBalance(address, amount.toString());
        return api.tx.sudo.sudo(fundTx);
    });
    const tx = api.tx.utility.batchAll(txs);
    return tx;
}

export async function waitEras(eras: number, api: ApiPromise, force = true) {
    if (force) {
        await forceNewEra(api);
    }
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
    await signSendAndWatch(sudoTx, api, initAliceKeyring());
}

export function randomTestAccount(secret = '') {
    if (secret === '') {
        secret = mnemonicGenerate();
    }
    const keyring = initKeyringPair(secret);
    const address = keyring.address;
    const evmAddress = substrateAddressToEvmAddress(address);
    return { secret, keyring, address, evmAddress };
}

export function initAliceKeyring() {
    return initKeyringPair('//Alice');
}

export async function randomFundedAccount(api: ApiPromise, sudoSigner: KeyringPair, amount: BN = parseAmount('1000')) {
    const account = randomTestAccount();
    const fundTx = await fundAddressesFromSudo([account.address], amount);
    await signSendAndWatch(fundTx, api, sudoSigner);
    return account;
}

export async function increaseValidatorCount(api: ApiPromise, sudoSigner: KeyringPair, additional = 3) {
    const oldCount = (await api.query.staking.validatorCount()).toNumber();

    await signSendAndWatch(api.tx.sudo.sudo(api.tx.staking.increaseValidatorCount(additional)), api, sudoSigner);

    const newCount = (await api.query.staking.validatorCount()).toNumber();
    expect(newCount).toEqual(oldCount + additional);
}

export function CLIBuilder(env: any) {
    let extraArgs = '';
    if (env.CC_PROXY_SECRET) {
        // WARNING: proxy setup must be done outside of this function
        const delegate = initKeyringPair(env.CC_SECRET);
        extraArgs = `--use-proxy ${delegate.address} --url ${BOB_NODE_URL}`;
    }

    function CLICmd(cmd: string) {
        return commandSync(`node ${CLI_PATH} ${cmd} ${extraArgs}`, { env });
    }
    return CLICmd;
}

export async function setUpProxy(nonProxiedCli: any, delegate: any, proxy: any, wrongProxy: any) {
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
            await fundFromSudo(proxy.address, new BN(0));
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

export async function setMinBondConfig(api: ApiPromise, value: number) {
    await setStakingConfig(initAliceKeyring(), api, null, value, null, null, null, null);
}
