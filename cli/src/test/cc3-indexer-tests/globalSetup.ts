const setup = () => {
    (global as any).CREDITCOIN_API_URL = 'ws://127.0.0.1:9944';
    (global as any).GRAPHQL_URL = 'http://127.0.0.1:3000';
};

export default setup;
