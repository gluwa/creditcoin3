import { signSendAndWatch } from '../../lib/tx';
import { initAliceKeyring, ALICE_NODE_URL, BOB_NODE_URL, randomFundedAccount, CLIBuilder } from './helpers';
import { ApiPromise, KeyringPair, newApi } from '../../lib';
import { randomEvmAccount } from './evmHelpers';
import { getEVMBalanceOf } from '../../lib/evm/balance';
import { convertWsToHttp } from '../../lib/evm/rpc';
import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { getBalance } from '../../lib/balance';
import { parseAmount } from '../../commands/options';
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

    describe('EVM Withdraw', () => {
        it.each([`--url ${ALICE_NODE_URL}`, `--url ${BOB_NODE_URL}`])(
            'should be able to withdraw CTC to a Substrate account via %s',
            async (nodeUrl) => {
                // Create a Substrate account
                const evmAddress = substrateAddressToEvmAddress(caller.address);

                // Fund associated EVM address
                const evmFundTX = api.tx.balances.forceSetBalance({ Address20: evmAddress }, parseAmount('100'));
                const evmFundSudoTX = api.tx.sudo.sudo(evmFundTX);
                await signSendAndWatch(evmFundSudoTX, api, initAliceKeyring());

                // Check that the EVM account has a balance
                const evmBalance = await getEVMBalanceOf(evmAddress, convertWsToHttp(ALICE_NODE_URL));
                expect(evmBalance.ctc).toBe(BigInt(parseAmount('100').toString()));

                // Withdraw 100 CTC to the Substrate account
                // requires the CC_SECRET set above
                CLI(`evm withdraw ${nodeUrl}`);

                // Check that the caller's Substrate account balance is greater than initial
                const afterBalance = await getBalance(caller.address, api);
                expect(BigInt(afterBalance.transferable.toString())).toBeGreaterThan(
                    BigInt(parseAmount('1000000').toString()),
                ); // Greater than initial balance
            },
            60000,
        );
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
            const fundingRes = CLI(`evm fund --evm-address ${evmAccount.address} --amount 100 --url ${BOB_NODE_URL}`);
            expect(fundingRes.exitCode).toBe(0);
            expect(fundingRes.stdout).toContain('Transaction included at block');

            const test2Res = CLI(`evm balance --evm-address ${evmAccount.address} --url ${BOB_NODE_URL}`);
            expect(test2Res.exitCode).toBe(0);
            expect(test2Res.stdout).toContain(' 100.0000 CTC');
        }, 100_000);
    });
});
