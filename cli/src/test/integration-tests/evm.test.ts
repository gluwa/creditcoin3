import { commandSync } from 'execa';
import { parseAmountInternal } from '../../lib/parsing';
import { signSendAndWatch } from '../../lib/tx';
import { randomTestAccount, fundAddressesFromSudo, initAliceKeyring, ALICE_NODE_URL, CLI_PATH } from './helpers';
import { newApi } from '../../lib';
import { randomEvmAccount } from './evmHelpers';
import { getEVMBalanceOf } from '../../lib/evm/balance';
import { convertWsToHttp } from '../../lib/evm/rpc';
import { evmAddressToSubstrateAddress, substrateAddressToEvmAddress } from '../../lib/evm/address';
import { getBalance } from '../../lib/balance';

describe('EVM commands', () => {
    it('should be able to fund an EVM account', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Create and fund a random Substrate account
        const caller = randomTestAccount();
        const fundTx = await fundAddressesFromSudo([caller.address], parseAmountInternal('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Create a random EVM account
        const evmAccount = randomEvmAccount();

        // Fund it with 10 CTC using the CLI
        const result = commandSync(
            `node ${CLI_PATH} evm fund --to ${evmAccount.address} --amount 10`,
            {
                env: {
                    CC_SECRET: caller.secret,
                },
            },
        );

        // Check that the transaction was included
        expect(result.stdout).toContain('Transaction included');

        // Check that the EVM account has a balance
        const evmBalance = await getEVMBalanceOf(evmAccount.address,convertWsToHttp(ALICE_NODE_URL));
        expect(evmBalance.ctc).toBeGreaterThan(0);

        await api.disconnect();
    }, 60000);
    it('should be able to send CTC between EVM accounts', async () =>
    {
        const { api } = await newApi(ALICE_NODE_URL);

         // Create two random EVM accounts
         const evmAccount1 = randomEvmAccount();
         const evmAccount2 = randomEvmAccount();

        // Create and fund one of them through its associated Substrate account
        const substrateAddress = evmAddressToSubstrateAddress(evmAccount1.address);
        const fundTx = await fundAddressesFromSudo([substrateAddress], parseAmountInternal('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Send 1 CTC from account 1 to account 2
        commandSync(
            `node ${CLI_PATH} evm send --to ${evmAccount2.address} --amount 1`,
            {
                env: {
                    EVM_SECRET: evmAccount1.mnemonic,
                },
            },
        );

        // Check that the second account balance is greater than 0
        const evmBalance2 = await getEVMBalanceOf(evmAccount2.address, convertWsToHttp(ALICE_NODE_URL));
        expect(evmBalance2.ctc).toBeGreaterThan(0);

        await api.disconnect();
    }, 60000);

    it('should be able to withdraw CTC to a Substrate account', async () =>
    {
        const { api } = await newApi(ALICE_NODE_URL);

        // Create one EVM account & a Substrate account
        const evmAccount = randomEvmAccount();
        const substrateAccount = randomTestAccount();

       // Create and fund the EVM account through its associated Substrate account
       const substrateAddress = evmAddressToSubstrateAddress(evmAccount.address);
       const fundTx = await fundAddressesFromSudo([substrateAddress], parseAmountInternal('10000'));
       await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Fund the Substrate account with 0.1 CTC to pay for fees
        const fundTx2 = await fundAddressesFromSudo([substrateAccount.address], parseAmountInternal('0.1'));
        await signSendAndWatch(fundTx2, api, initAliceKeyring());

        // Send 1 CTC from the EVM account to the Substrate account
        const associatedEvmAccount = substrateAddressToEvmAddress(substrateAccount.address);
        commandSync(
            `node ${CLI_PATH} evm send --to ${associatedEvmAccount} --amount 1`,
            {
                env: {
                    EVM_SECRET: evmAccount.mnemonic,
                },
            },
        );

        // Withdraw 1 CTC to the Substrate account
        commandSync(
            `node ${CLI_PATH} evm withdraw`,
            {
                env: {
                    CC_SECRET: substrateAccount.secret,
                },
            },
        );
   
        // Check that the caller's Substrate account balance is greater than 1
        const balance = await getBalance(substrateAccount.address, api);
        expect(BigInt(balance.total.toString())).toBeGreaterThan(1);

        await api.disconnect();
    }, 60000);
});
