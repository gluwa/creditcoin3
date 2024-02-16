import { signSendAndWatch } from '../../lib/tx';
import { fundAddressesFromSudo, initAliceKeyring, ALICE_NODE_URL } from './helpers';
import { ApiPromise, KeyringPair, MICROUNITS_PER_CTC, newApi } from '../../lib';
import { randomEvmAccount } from './evmHelpers';
import { getEVMBalanceOf } from '../../lib/evm/balance';
import { convertWsToHttp } from '../../lib/evm/rpc';
import { evmAddressToSubstrateAddress, substrateAddressToEvmAddress } from '../../lib/evm/address';
import { getBalance } from '../../lib/balance';
import { parseAmount } from '../../commands/options';
import { randomFundedAccount, CLIBuilder } from './helpers';
import { describeIf } from '../utils';

describeIf(process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no', 'EVM Commands', () => {
    let api: ApiPromise;
    let caller: { secret: any; keyring: KeyringPair; address: string };
    let CLI: (arg0: string) => any;

    beforeEach(async () => {
        caller = await randomFundedAccount(api, initAliceKeyring(), parseAmount('1000000'));
        CLI = CLIBuilder({ CC_SECRET: caller.secret });
    }, 100_000);

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));
    }, 100_000);

    afterAll(async () => {
        await api.disconnect();
    }, 10_000);

    describe('EVM Fund', () => {
        it('should be able to fund an EVM account', async () => {
            // Create a random EVM account
            const evmAccount = randomEvmAccount();

            const result = CLI(`evm fund --evm-address ${evmAccount.address} --amount 10`);

            // Check that the transaction was included
            expect(result.stdout).toContain('Transaction included');

            // Check that the EVM account has a balance
            const evmBalance = await getEVMBalanceOf(evmAccount.address, convertWsToHttp(ALICE_NODE_URL));
            expect(evmBalance.ctc).toBeGreaterThan(0);
        }, 60000);

        it('should not be able to fund more than existing funds', () => {
            const evmAccount = randomEvmAccount();

            try {
                CLI(`evm fund --evm-address ${evmAccount.address} --amount 1000000`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(`has insufficient funds to send the transaction`);
            }
        }, 100_000);
    });

    describe('EVM Send', () => {
        it('should be able to send CTC between EVM accounts', async () => {
            // Create two random EVM accounts
            const evmAccount1 = randomEvmAccount();
            const evmAccount2 = randomEvmAccount();

            // Create and fund one of them through its associated Substrate account
            const substrateAddress = evmAddressToSubstrateAddress(evmAccount1.address);
            const fundTx = await fundAddressesFromSudo([substrateAddress], parseAmount('10000'));
            await signSendAndWatch(fundTx, api, initAliceKeyring());

            // override the default CLI instance with one capable of making evm commands
            const CLI2 = CLIBuilder({ EVM_SECRET: evmAccount1.mnemonic });
            CLI2(`evm send --evm-address ${evmAccount2.address} --amount 1`);

            // Check that the second account balance is greater than 0
            const evmBalance2 = await getEVMBalanceOf(evmAccount2.address, convertWsToHttp(ALICE_NODE_URL));
            const expectedBalance = BigInt(parseAmount('1').toString());
            expect(evmBalance2.ctc).toBe(expectedBalance);
        }, 60000);
    });

    describe('EVM Withdraw', () => {
        it('should be able to withdraw CTC to a Substrate account', async () => {
            // Create one EVM account & a Substrate account
            const evmAccount = randomEvmAccount();

            // Create and fund the EVM account through its associated Substrate account
            const substrateAddress = evmAddressToSubstrateAddress(evmAccount.address);
            const fundTx = await fundAddressesFromSudo([substrateAddress], parseAmount('10000'));
            await signSendAndWatch(fundTx, api, initAliceKeyring());

            // Send 1 CTC from the EVM account to the Substrate account
            const associatedEvmAccount = substrateAddressToEvmAddress(caller.address);

            // override the default CLI instance with one capable of making evm and substrate commands
            const CLI2 = CLIBuilder({ EVM_SECRET: evmAccount.mnemonic, CC_SECRET: caller.secret });
            CLI2(`evm send --evm-address ${associatedEvmAccount} --amount 1`);

            // Withdraw 1 CTC to the Substrate account
            // requires the CC_SECRET set above
            CLI2(`evm withdraw`);

            // Check that the caller's Substrate account balance is greater than 1
            const balance = await getBalance(caller.address, api);
            expect(BigInt(balance.total.toString())).toBeGreaterThan(1 * MICROUNITS_PER_CTC); // 1 CTC
        }, 60000);
    });

    describe('EVM Balance', () => {
        it('should be able to show evm balance correctly when balance is zero', () => {
            // create evm account
            const evmAccount = randomEvmAccount();

            // Can correctly see a zero balance for an unfunded account
            const test1Res = CLI(`evm balance --evm-address ${evmAccount.address}`);
            expect(test1Res.exitCode).toBe(0);
            expect(test1Res.stdout).toContain('0.0000');
        }, 300_000);

        it('should be able to show balance correctly after funding', () => {
            // create evm account
            const evmAccount = randomEvmAccount();

            // Create and fund a random Substrate account
            const fundingRes = CLI(`evm fund --evm-address ${evmAccount.address} --amount 100`);
            expect(fundingRes.exitCode).toBe(0);
            expect(fundingRes.stdout).toContain('Transaction included at block');

            const test2Res = CLI(`evm balance --evm-address ${evmAccount.address}`);
            expect(test2Res.exitCode).toBe(0);
            expect(test2Res.stdout).toContain(' 100.0000 CTC');
        }, 100_000);
    });
});
