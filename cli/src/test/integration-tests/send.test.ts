import { commandSync } from 'execa';
import { newApi } from '../../api';
import { parseAmountInternal } from '../../lib/parsing';
import { signSendAndWatch } from '../../lib/tx';
import {
    randomTestAccount,
    fundAddressesFromSudo,
    initAlithKeyring,
} from './helpers';

describe('Send command', () => {
    it('should be able to send CTC when %s', async () => {
        const { api } = await newApi();

        const caller = randomTestAccount();

        const fundTx = await fundAddressesFromSudo(
            [caller.address],
            parseAmountInternal('10000')
        );
        await signSendAndWatch(fundTx, api, initAlithKeyring());

        const result = commandSync(
            `node dist/index.js send --to 5HDRB6edmWwwh6aCDKrRSbisV8iFHdP7jDy18U2mt9w2wEkq --amount 10`,
            {
                env: {
                    CC_SECRET: caller.secret,
                },
            }
        );

        expect(result.stdout).toContain('Transaction included');
        await api.disconnect();
    }, 60000);
});
