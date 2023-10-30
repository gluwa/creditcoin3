import { cryptoWaitReady } from "@polkadot/util-crypto";
import { Command, OptionValues } from "commander";
import { initCallerKeyring } from "../lib/account/keyring";
import { blake2AsHex, decodeAddress } from "@polkadot/util-crypto";
import { u8aToHex } from "@polkadot/util"

export function makeShowAddressCommand() {
  const cmd = new Command("show-address");
  cmd.description("Show account address");
  cmd.action(showAddressAction);
  return cmd;
}

async function showAddressAction(options: OptionValues) {
  await cryptoWaitReady();
  
  const caller = await initCallerKeyring(options);
  const substrateAddress = caller.address;
  const substrateAddressBytes = decodeAddress(substrateAddress);


  const evmAddress = u8aToHex(substrateAddressBytes).slice(0,42);

  console.log("Account Substrate Address:", substrateAddress);
  console.log("Account EVM Address:", evmAddress);


  process.exit(0);
}
