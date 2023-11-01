import { ApiPromise } from "@polkadot/api";

export interface CreditcoinApi {
  api: ApiPromise;
  // extrinsics: Extrinsics;
  // utils: { signAccountId: (signer: Wallet, accountId: AccountId) => string };
}
