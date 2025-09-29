import { buildModule } from '@nomicfoundation/hardhat-ignition/modules';

const ProverModule = buildModule('ProverModule', (m) => {
    const proceedsAccount = '0x0000000000000000000000000000000000000000';
    const costPerByte = 10n;
    const baseFee = 1000n;
    const chainKey = 0;
    const displayName = 'From-Ignition';
    const timeout = 10 * 2; // num blocks * block time

    const contract = m.contract('CreditcoinPublicProver', [
        proceedsAccount,
        costPerByte,
        baseFee,
        chainKey,
        displayName,
        timeout,
    ]);

    return { contract };
});

export default ProverModule;
