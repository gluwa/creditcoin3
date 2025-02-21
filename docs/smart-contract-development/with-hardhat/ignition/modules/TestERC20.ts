import { buildModule } from '@nomicfoundation/hardhat-ignition/modules';

const TestERC20Module = buildModule('TestERC20Module', (m) => {
    const counter = m.contract('Counter');

    return { counter };
});

export default TestERC20Module;