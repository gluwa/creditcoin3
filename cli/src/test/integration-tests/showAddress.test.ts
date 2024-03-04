import { commandSync } from 'execa';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { CLI_PATH, randomTestAccount, ALICE_NODE_URL, BOB_NODE_URL } from './helpers';
import { describeIf } from '../utils';
import { parseSubstrateAddress, parseEVMAddress } from '../../commands/options';
import { substrateAddressToEvmAddress } from '../../lib/evm/address';

describeIf(
    process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
    'Show address command',
    () => {
        it.each([`--url ${ALICE_NODE_URL}`, `--url ${BOB_NODE_URL}`])(
            'should return the correct address via %s',
            async (nodeUrl) => {
                await cryptoWaitReady();

                const caller = randomTestAccount();

                const result = commandSync(`node ${CLI_PATH} show-address ${nodeUrl}`, {
                    env: {
                        CC_SECRET: caller.secret,
                    },
                }).stdout;

                const substrateAddress = parseSubstrateAddress(
                    result
                        .split(/\r?\n/)[0] // First line of the output
                        .split('Account Substrate address: ')[1], // Substrate address
                );

                const evmAddress = parseEVMAddress(
                    result
                        .split(/\r?\n/)[1] // Second line of the output
                        .split('Associated EVM address: ')[1], // EVM address
                );

                expect(substrateAddress).toEqual(caller.address.toString());
                expect(evmAddress).toEqual(substrateAddressToEvmAddress(caller.address));
            },
            60000,
        );
    },
);
