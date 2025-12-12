import { expect } from "chai";
import { ethers } from "hardhat";

describe("Groth16Verifier precompile integration", function () {
    it("verifies the known Groth16 proof", async function () {
        // Deploy the verifier contract
        const Verifier = await ethers.getContractFactory("Groth16Verifier");
        const verifier = await Verifier.deploy();

        // These are exactly the values from your Solidity test
        const pA: [string, string] = [
            "0x0df8bba2c810bc8d82b2b993d551e23bc283793d8877fa9f350f2879caac0fbe",
            "0x10b527d51a7406d500c2c9f740c68b299497b0af91eef8e440d203bca63347e6",
        ];

        const pB: [[string, string], [string, string]] = [
            [
                "0x1db5e8bb9860bd36f86c7a9e285668fc81a8fe4c45ce821fb6596250a893baeb",
                "0x1bc41a91a86102c746eaa6500079b563fd905885d2cb8c22846b5386edb27524",
            ],
            [
                "0x229ac7e8007da3e6de10c906147cbe8b559f72e81e4fef8815ebfefad786a6ab",
                "0x1873b49f6a1725d5c75126b35b10eb1bae001a5593b194d1b27a0655bc02d9e4",
            ],
        ];

        const pC: [string, string] = [
            "0x10fb48e035fe9edb0a56bce95b2d5a0b12d79afe8e01a80fbcebbd2bcc1c9b97",
            "0x1bd237fa61110f1030092f3610edde7196f3729da67de43a8f1609a76a772dab",
        ];

        const pubSignals: [string] = [
            "0x0000000000000000000000000000000000000000000000000000000000000021",
        ];

        const result = await verifier.verifyProof(pA, pB, pC, pubSignals);
        expect(result).to.equal(true);
    });
});
