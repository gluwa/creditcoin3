import { Command, OptionValues } from "commander";
import { newApi } from "../../api";
import { BN } from "../../lib"
import { bond, checkRewardDestination } from "../../lib/staking";
import { promptContinue, setInteractivity } from "../../lib/interactive";
import {
  AccountBalance,
  getBalance,
  toCTCString,
  checkAmount,
} from "../../lib/balance";

import {
  inputOrDefault,
  parseAddressOrExit,
  parseAmountOrExit,
  parseBoolean,
  parseChoiceOrExit,
  requiredInput,
} from "../../lib/parsing";
import { initStashKeyring } from "../../lib/account/keyring";

export function makeBondCommand() {
  const cmd = new Command("bond");
  cmd.description("Bond CTC from a Stash account");
  cmd.option("-a, --amount [amount]", "Amount to bond");
  cmd.option(
    "-r, --reward-destination [reward-destination]",
    "Specify reward destination account to use for new account",
  );
  cmd.option(
    "-x, --extra",
    "Bond as extra, adding more funds to an existing bond",
  );
  cmd.action(bondAction);
  return cmd;
}

async function bondAction(options: OptionValues) {
  const { api } = await newApi(options.url);

  const { amount, rewardDestination, extra, interactive } =
    parseOptions(options);

  const stashKeyring = await initStashKeyring(options);
  const stashAddress = stashKeyring.address;

  // Check if stash has enough balance
  await checkBalance(amount, api, stashAddress);

  console.log("Creating bond transaction...");
  console.log("Reward destination:", rewardDestination);
  console.log("Amount:", toCTCString(amount));
  if (extra) {
    console.log("Bonding as 'extra'; funds will be added to existing bond");
  }

  await promptContinue(interactive);

  const bondTxResult = await bond(
    stashKeyring,
    amount,
    rewardDestination,
    api,
    extra,
  );

  console.log(bondTxResult.info);
  process.exit(0);
}

async function checkBalance(amount: BN, api: any, address: string) {
  const balance = await getBalance(address, api);
  checkBalanceAgainstBondAmount(balance, amount);
}

function checkBalanceAgainstBondAmount(balance: AccountBalance, amount: BN) {
  if (balance.transferable.lt(amount)) {
    console.error(
      `Insufficient funds to bond ${toCTCString(amount)}, only ${toCTCString(
        balance.transferable,
      )} available`,
    );
    process.exit(1);
  }
}

function parseOptions(options: OptionValues) {
  const amount = parseAmountOrExit(
    requiredInput(
      options.amount,
      "Failed to bond: Must specify an amount to bond",
    ),
  );
  checkAmount(amount);

  const rewardDestination = checkRewardDestination(
    parseChoiceOrExit(inputOrDefault(options.rewardDestination, "Staked"), [
      "Staked",
      "Stash",
    ]),
  );

  const extra = parseBoolean(options.extra);

  const interactive = setInteractivity(options);

  return { amount, rewardDestination, extra, interactive };
}
