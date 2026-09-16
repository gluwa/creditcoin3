"use client";

import { useMemo, useState } from "react";
import { useConfig } from "wagmi";
import { getPublicClient } from "wagmi/actions";
import { formatEther, formatUnits, isHash, maxUint256 } from "viem";

import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { HashChip } from "@/app/components/hash-chip";
import { Metric } from "@/app/components/metric";
import { SPOKE_CHAINS, spokeByChainKey } from "@/lib/chains";
import {
  BRIDGE_HUB_ADDRESS,
  creditcoinPublicClient,
} from "@/lib/contracts/hub";
import { BridgeHubAbi } from "@/lib/contracts/generated/bridge-hub.abi";
import { BridgeVaultAbi } from "@/lib/contracts/generated/bridge-vault.abi";
import { useHubInspect, useVaultInspect } from "@/hooks/use-contract-inspect";
import { useQuery } from "@tanstack/react-query";

// approveOutboxFee's default is `ethers.MaxUint256` (fund-bridge-hub.mts: "approve once, never
// repeat for this outbox") — a deliberate unlimited approval, not a bug. Formatted as a plain
// token amount that's a 60+ digit number, so special-case it the way wallets/explorers do.
function formatAllowance(
  value: bigint,
  decimals: number,
  symbol: string,
): string {
  if (value === maxUint256) return "Unlimited";
  return `${formatUnits(value, decimals)} ${symbol}`;
}

function BoolBadge({
  value,
  trueLabel,
  falseLabel,
}: {
  value: boolean | undefined;
  trueLabel: string;
  falseLabel: string;
}) {
  if (value === undefined) return <Badge variant="secondary">Unknown</Badge>;
  return value ? (
    <Badge variant="success">{trueLabel}</Badge>
  ) : (
    <Badge variant="destructive">{falseLabel}</Badge>
  );
}

function HubCard() {
  const { available, data, isLoading, error } = useHubInspect();
  const [claimKeyInput, setClaimKeyInput] = useState("");

  const claimedLookup = useQuery({
    queryKey: ["inspect-claimed-lookup", claimKeyInput],
    queryFn: async () => {
      if (!creditcoinPublicClient || !BRIDGE_HUB_ADDRESS) return null;
      return creditcoinPublicClient.readContract({
        address: BRIDGE_HUB_ADDRESS,
        abi: BridgeHubAbi,
        functionName: "claimed",
        args: [claimKeyInput as `0x${string}`],
      });
    },
    enabled: Boolean(
      creditcoinPublicClient && BRIDGE_HUB_ADDRESS && isHash(claimKeyInput),
    ),
  });

  if (!available) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>BridgeHub</CardTitle>
          <CardDescription>
            Not configured — set NEXT_PUBLIC_BRIDGE_HUB_ADDRESS and
            NEXT_PUBLIC_CREDITCOIN_RPC_URL to inspect it.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>BridgeHub</CardTitle>
        <CardDescription>Creditcoin</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {isLoading && !data && (
          <p className="text-sm text-muted-foreground">Loading...</p>
        )}
        {error && (
          <p className="text-sm text-destructive">
            Failed to read BridgeHub: {error.message}
          </p>
        )}
        {data && (
          <>
            <div>
              <Metric
                label="Address"
                value={<HashChip hash={data.address} kind="address" />}
              />
              <Metric
                label="Owner"
                value={<HashChip hash={data.owner} kind="address" />}
              />
              <Metric
                label="Status"
                value={
                  data.paused ? (
                    <Badge variant="destructive">Paused</Badge>
                  ) : (
                    <Badge variant="success">Active</Badge>
                  )
                }
              />
              <Metric
                label="Proof verifier"
                value={<HashChip hash={data.proofVerifier} kind="address" />}
              />
              <Metric
                label="ATTEST balance"
                value={`${formatUnits(data.attestToken.hubBalance, data.attestToken.decimals)} ${data.attestToken.symbol}`}
              />
            </div>

            <Separator />

            <div className="flex flex-col gap-3">
              <span className="text-sm font-medium">
                Configured chains ({data.chains.length})
              </span>
              {data.chains.length === 0 && (
                <p className="text-sm text-muted-foreground">
                  No spokes configured in this dApp build.
                </p>
              )}
              {data.chains.map((chain) => (
                <div
                  key={chain.chainKey}
                  className="flex flex-col gap-1 rounded-md border p-3 text-sm"
                >
                  <div className="flex items-center justify-between">
                    <span className="font-medium">
                      {chain.chainName} (key {chain.chainKey})
                    </span>
                    <BoolBadge
                      value={chain.enabled}
                      trueLabel="Enabled"
                      falseLabel="Disabled"
                    />
                  </div>
                  <Metric
                    label="Vault"
                    value={<HashChip hash={chain.vault} kind="address" />}
                  />
                  <Metric
                    label="Outbox"
                    value={<HashChip hash={chain.outbox} kind="address" />}
                  />
                  <Metric
                    label="ATTEST allowance to outbox"
                    value={formatAllowance(
                      chain.outboxAttestAllowance,
                      data.attestToken.decimals,
                      data.attestToken.symbol,
                    )}
                  />
                </div>
              ))}
            </div>

            <Separator />

            <div className="flex flex-col gap-2">
              <Label htmlFor="claim-key">Look up a claim key</Label>
              <div className="flex gap-2">
                <Input
                  id="claim-key"
                  placeholder="0x... (keccak256(sourceChainKey, vault, nonce))"
                  value={claimKeyInput}
                  onChange={(e) => setClaimKeyInput(e.target.value)}
                />
              </div>
              {isHash(claimKeyInput) && (
                <p className="text-sm">
                  {claimedLookup.isLoading
                    ? "Checking..."
                    : claimedLookup.data
                      ? "✅ Already claimed"
                      : "Not claimed"}
                </p>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}

function VaultCard() {
  const wagmiConfig = useConfig();
  const [chainKey, setChainKey] = useState(SPOKE_CHAINS[0]?.chainKey);
  const spoke = useMemo(
    () => (chainKey !== undefined ? spokeByChainKey(chainKey) : undefined),
    [chainKey],
  );
  const { data, isLoading, error } = useVaultInspect(spoke);
  const [messageIdInput, setMessageIdInput] = useState("");

  const processedLookup = useQuery({
    queryKey: ["inspect-processed-lookup", spoke?.chainKey, messageIdInput],
    queryFn: async () => {
      if (!spoke) return null;
      const client = getPublicClient(wagmiConfig, { chainId: spoke.chain.id });
      if (!client) return null;
      return client.readContract({
        address: spoke.bridgeVaultAddress,
        abi: BridgeVaultAbi,
        functionName: "processedMessages",
        args: [messageIdInput as `0x${string}`],
      });
    },
    enabled: Boolean(spoke && isHash(messageIdInput)),
  });

  if (SPOKE_CHAINS.length === 0) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>BridgeVault</CardTitle>
          <CardDescription>
            No spokes configured in this dApp build.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>BridgeVault</CardTitle>
        <CardDescription>
          <Select
            value={chainKey !== undefined ? String(chainKey) : undefined}
            onValueChange={(v) => setChainKey(Number(v))}
          >
            <SelectTrigger className="mt-1 w-[220px]">
              <SelectValue placeholder="Select a spoke" />
            </SelectTrigger>
            <SelectContent>
              {SPOKE_CHAINS.map((s) => (
                <SelectItem key={s.chainKey} value={String(s.chainKey)}>
                  {s.chain.name} (key {s.chainKey})
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {isLoading && !data && (
          <p className="text-sm text-muted-foreground">Loading...</p>
        )}
        {error && (
          <p className="text-sm text-destructive">
            Failed to read BridgeVault: {error.message}
          </p>
        )}
        {data && spoke && (
          <>
            <div>
              <Metric
                label="Address"
                value={
                  <HashChip
                    hash={data.address}
                    kind="address"
                    chain={spoke.chain}
                  />
                }
              />
              <Metric
                label="Owner"
                value={
                  <HashChip
                    hash={data.owner}
                    kind="address"
                    chain={spoke.chain}
                  />
                }
              />
              <Metric
                label="Native balance"
                value={`${formatEther(data.nativeBalance)} ${spoke.chain.nativeCurrency.symbol}`}
              />
              <Metric
                label="Deposit nonce"
                value={data.depositNonce.toString()}
              />
              <Metric
                label="BridgeHub"
                value={
                  <span className="flex items-center gap-2">
                    <HashChip hash={data.bridgeHub} kind="address" />
                    <BoolBadge
                      value={data.bridgeHubMatchesConfigured}
                      trueLabel="Matches config"
                      falseLabel="Mismatch!"
                    />
                  </span>
                }
              />
              <Metric
                label="Trusts configured router"
                value={
                  <BoolBadge
                    value={data.trustsConfiguredRouter}
                    trueLabel="Trusted"
                    falseLabel="Not trusted"
                  />
                }
              />
            </div>

            <Separator />

            <div className="flex flex-col gap-2">
              <Label htmlFor="message-id">Look up a release messageId</Label>
              <div className="flex gap-2">
                <Input
                  id="message-id"
                  placeholder="0x... (from BridgeHub's Claimed event)"
                  value={messageIdInput}
                  onChange={(e) => setMessageIdInput(e.target.value)}
                />
              </div>
              {isHash(messageIdInput) && (
                <p className="text-sm">
                  {processedLookup.isLoading
                    ? "Checking..."
                    : processedLookup.data
                      ? "✅ Processed (released, or replay-blocked)"
                      : "Not yet processed"}
                </p>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}

export default function InspectPage() {
  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-semibold">Inspect contracts</h1>
        <p className="text-sm text-muted-foreground">
          Read-only view of the deployed BridgeHub and BridgeVault contracts —
          useful for debugging a stuck or failed transfer.
        </p>
      </div>
      <HubCard />
      <VaultCard />
    </div>
  );
}
