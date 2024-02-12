import { signSendAndWatch } from '../../lib/tx';
import { randomTestAccount, fundAddressesFromSudo, initAliceKeyring, ALICE_NODE_URL } from './helpers';
import { MICROUNITS_PER_CTC, newApi } from '../../lib';
import { randomEvmAccount } from './evmHelpers';
import { getEVMBalanceOf } from '../../lib/evm/balance';
import { convertWsToHttp } from '../../lib/evm/rpc';
import { evmAddressToSubstrateAddress, substrateAddressToEvmAddress } from '../../lib/evm/address';
import { getBalance } from '../../lib/balance';
import { parseAmount } from '../../commands/options';
import { randomFundedAccount, CLIBuilder } from './helpers';

describe('EVM commands', () => {
    it('should be able to fund an EVM account', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Create and fund a random Substrate account
        const caller = randomTestAccount();
        const fundTx = await fundAddressesFromSudo([caller.address], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Create a random EVM account
        const evmAccount = randomEvmAccount();

        const CLI = CLIBuilder({ env: { CC_SECRET: caller.secret } });
        const result = CLI(`evm fund --evm-address ${evmAccount.address} --amount 10`);

        // Check that the transaction was included
        expect(result.stdout).toContain('Transaction included');

        // Check that the EVM account has a balance
        const evmBalance = await getEVMBalanceOf(evmAccount.address, convertWsToHttp(ALICE_NODE_URL));
        expect(evmBalance.ctc).toBeGreaterThan(0);

        await api.disconnect();
    }, 60000);
    it('should be able to send CTC between EVM accounts', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Create two random EVM accounts
        const evmAccount1 = randomEvmAccount();
        const evmAccount2 = randomEvmAccount();

        // Create and fund one of them through its associated Substrate account
        const substrateAddress = evmAddressToSubstrateAddress(evmAccount1.address);
        const fundTx = await fundAddressesFromSudo([substrateAddress], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        const CLI = CLIBuilder({ env: { EVM_SECRET: evmAccount1.mnemonic } });
        CLI(`evm send --evm-address ${evmAccount2.address} --amount `);

        // Check that the second account balance is greater than 0
        const evmBalance2 = await getEVMBalanceOf(evmAccount2.address, convertWsToHttp(ALICE_NODE_URL));

        const expectedBalance =
            BigInt(parseAmount('1').toString()) - BigInt(api.consts.balances.existentialDeposit.toBigInt()); // Remove existential amount from the expected balance
        expect(evmBalance2.ctc).toBe(expectedBalance);

        await api.disconnect();
    }, 60000);

    it('should be able to withdraw CTC to a Substrate account', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Create one EVM account & a Substrate account
        const evmAccount = randomEvmAccount();
        const substrateAccount = randomTestAccount();

        const CLI = CLIBuilder({ CC_SECRET: substrateAccount.secret });

        // Create and fund the EVM account through its associated Substrate account
        const substrateAddress = evmAddressToSubstrateAddress(evmAccount.address);
        const fundTx = await fundAddressesFromSudo([substrateAddress], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Fund the Substrate account with 0.1 CTC to pay for fees
        const fundTx2 = await fundAddressesFromSudo([substrateAccount.address], parseAmount('0.1'));
        await signSendAndWatch(fundTx2, api, initAliceKeyring());

        // Send 1 CTC from the EVM account to the Substrate account
        const associatedEvmAccount = substrateAddressToEvmAddress(substrateAccount.address);
        CLI(`evm send --evm-address ${associatedEvmAccount} --amount 1`);

        // Withdraw 1 CTC to the Substrate account
        CLI(`evm withdraw`);

        // Check that the caller's Substrate account balance is greater than 1
        const balance = await getBalance(substrateAccount.address, api);
        expect(BigInt(balance.total.toString())).toBeGreaterThan(1 * MICROUNITS_PER_CTC); // 1 CTC

        await api.disconnect();
    }, 60000);

    it('should be able to show evm balance correctly when balance is zero', () => {
        const caller = randomTestAccount();
        const CLI = CLIBuilder({ CC_SECRET: caller.secret });

        // create evm account
        const evmAccount = randomEvmAccount();

        // Can correctly see a zero balance for an unfunded account
        const test1Res = CLI(`evm balance --evm-address ${evmAccount.address}`);
        expect(test1Res.exitCode).toBe(0);
        expect(test1Res.stdout).toContain('0.0000');
    }, 300_000);

    it('should be able to show balance correctly after funding', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        const caller = await randomFundedAccount(api, initAliceKeyring(), parseAmount('1000000'));
        const CLI = CLIBuilder({ CC_SECRET: caller.secret });

        // create evm account
        const evmAccount = randomEvmAccount();

        // Create and fund a random Substrate account
        const fundingRes = CLI(`evm fund --evm-address ${evmAccount.address} --amount 100`);
        expect(fundingRes.exitCode).toBe(0);
        expect(fundingRes.stdout).toContain('Transaction included at block');

        const test2Res = CLI(`evm balance --evm-address ${evmAccount.address}`);
        expect(test2Res.exitCode).toBe(0);
        expect(test2Res.stdout).toContain(' 99.9999 CTC');
    }, 100_000);

    it('should not be able to fund more than existing funds', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        const caller = await randomFundedAccount(api, initAliceKeyring(), parseAmount('100'));
        const CLI = CLIBuilder({ CC_SECRET: caller.secret });

        // create evm account
        const evmAccount = randomEvmAccount();

        expect(CLI(`evm fund --evm-address ${evmAccount.address} --amount 1000000`)).toThrow(Error);
    }, 100_000);
});
