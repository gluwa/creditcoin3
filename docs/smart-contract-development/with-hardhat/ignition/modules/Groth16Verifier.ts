import { buildModule } from "@nomicfoundation/hardhat-ignition/modules";

const Groth16VerifierModule = buildModule("Groth16VerifierModule", (m) => {
    // Deploy the contract (no constructor params)
    const verifier = m.contract("Groth16Verifier");

    // Export it so other modules / scripts can use the address
    return { verifier };
});

export default Groth16VerifierModule;
