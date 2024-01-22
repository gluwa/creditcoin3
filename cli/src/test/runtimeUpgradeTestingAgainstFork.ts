import { default as globalSetup } from './blockchainSetup';

const setup = () => {
    (global as any).CREDITCOIN_USES_FAST_RUNTIME = false;
    (global as any).CREDITCOIN_HAS_EVM_TRACING = false;

    globalSetup();
};

export default setup;
