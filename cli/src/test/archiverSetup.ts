export const creditcoinApiUrl = (defaultUrl: string) => {
    return process.env.CREDITCOIN_API_URL || defaultUrl;
};

const setup = () => {
    process.env.NODE_ENV = 'test';

    if ((global as any).CREDITCOIN_API_URL === undefined) {
        const wsPort = process.env.CREDITCOIN_WS_PORT || '9944';
        (global as any).CREDITCOIN_API_URL = creditcoinApiUrl(`ws://127.0.0.1:${wsPort}`);
    }

    if ((global as any).ARCHIVER_URL === undefined) {
        (global as any).ARCHIVER_URL = 'http://127.0.0.1:8080';
    }
};

export default setup;
