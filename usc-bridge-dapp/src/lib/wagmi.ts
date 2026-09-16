import { getDefaultConfig } from "@rainbow-me/rainbowkit";
import {
  injectedWallet,
  metaMaskWallet,
  rainbowWallet,
  walletConnectWallet,
} from "@rainbow-me/rainbowkit/wallets";

import { WAGMI_CHAINS } from "./chains";

// getDefaultConfig throws at module-eval time (i.e. build time, not just runtime) on an empty
// projectId — fine for a real deployment that sets the secret, but it means a bare `yarn build`
// (CI, or a fresh local checkout before .env.local exists) hard-fails instead of producing a build
// that merely can't offer WalletConnect. A placeholder keeps the build green; only the WalletConnect
// QR-code option actually needs a real one, injected wallets (MetaMask, etc.) don't.
const projectId = process.env.NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID;
if (!projectId) {
  console.warn(
    "NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID is unset — WalletConnect will not work. " +
      "Get a project id from https://cloud.walletconnect.com and set it in .env.local.",
  );
}

// Explicit wallet list rather than getDefaultConfig's full default set: the default list includes
// Coinbase Smart Wallet (@wagmi/connectors' baseAccount), which transitively pulls in
// @coinbase/cdp-sdk's optional x402 payment-protocol dynamic imports — packages this bridge never
// installs or needs, and Next.js's build fails trying to statically resolve them. A handful of
// well-supported wallets covers this POC's actual testing needs.
export const wagmiConfig = getDefaultConfig({
  appName: "Creditcoin Bridge (POC)",
  projectId: projectId || "00000000000000000000000000000000",
  chains: WAGMI_CHAINS,
  wallets: [
    {
      groupName: "Recommended",
      wallets: [
        metaMaskWallet,
        rainbowWallet,
        walletConnectWallet,
        injectedWallet,
      ],
    },
  ],
  ssr: true,
});
