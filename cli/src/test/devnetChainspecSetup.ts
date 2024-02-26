import { default as globalSetup } from './blockchainSetup';

const setup = () => {
    (global as any).CREDITCOIN_EXPECTED_EPOCH_DURATION = 2880;
    (global as any).CREDITCOIN_EXPECTED_BLOCK_TIME = 15000;
    (global as any).CREDITCOIN_EXPECTED_MINIMUM_PERIOD = 7500;

    globalSetup();
};

export default setup;
