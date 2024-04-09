const { ethers } = require("hardhat");

async function main() {
  // Get signer
  const [signer] = await ethers.getSigners();

  // Specify the recipient address
  const recipientAddress = "0x92d3267215Ec56542b985473E73C8417403B15ac"; // Replace this with the recipient's address

  // Perform the transfer
  const tx = await signer.sendTransaction({
    to: recipientAddress,
    value: BigInt(1*1e18),
  });

  // Wait for the transaction to be mined
  await tx.wait();

  console.log(`Transfer successful! 1 ETH has been sent to ${recipientAddress} ${tx.hash}, ${tx.blockNumber}`);
}

// Execute the function
main()
  .then(() => process.exit(0))
  .catch(error => {
    console.error(error);
    process.exit(1);
  });