import { default as globalSetup } from './blockchainSetup';

const setup = () => {
    (global as any).CREDITCOIN_USES_FAST_RUNTIME = false;

    globalSetup();
};

export default setup;
