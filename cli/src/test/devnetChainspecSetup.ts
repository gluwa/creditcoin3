import { default as globalSetup } from './blockchainSetup';

const setup = () => {
    // --features "fast-runtime devnet"
    (global as any).CREDITCOIN_EXPECTED_EPOCH_DURATION = 1440;
    (global as any).CREDITCOIN_EXPECTED_BLOCK_TIME = 5000;
    (global as any).CREDITCOIN_EXPECTED_MINIMUM_PERIOD = 720;

    globalSetup();
};

export default setup;
