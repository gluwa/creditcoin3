"use client";

import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  useAccount,
  useChainId,
  useWaitForTransactionReceipt,
  useWriteContract,
} from "wagmi";
import { parseEther, zeroAddress } from "viem";

import { Button } from "@/components/ui/button";
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
import { SPOKE_CHAINS, spokeByEvmChainId } from "@/lib/chains";
import { BridgeVaultAbi } from "@/lib/contracts/generated/bridge-vault.abi";
import { useTransferStore } from "@/lib/transfer-store";

export default function DepositPage() {
  const router = useRouter();
  const { address, isConnected } = useAccount();
  const chainId = useChainId();
  const source = spokeByEvmChainId(chainId);
  const addTransfer = useTransferStore((s) => s.addTransfer);

  const destinations = useMemo(
    () => SPOKE_CHAINS.filter((s) => s.chainKey !== source?.chainKey),
    [source],
  );

  // Derived defaults (recipient = self, destination = the first available spoke) rather than
  // synced via effect: an effect that calls setState synchronously from a prop it can just read
  // directly only adds a redundant render.
  const [destChainKeyInput, setDestChainKeyInput] = useState("");
  const [amount, setAmount] = useState("");
  const [recipientInput, setRecipientInput] = useState("");

  const destChainKey =
    destChainKeyInput || String(destinations[0]?.chainKey ?? "");
  const recipient = recipientInput || address || "";

  const { writeContract, data: hash, isPending, error } = useWriteContract();
  const { isLoading: isConfirming, isSuccess } = useWaitForTransactionReceipt({
    hash,
  });

  useEffect(() => {
    if (
      !isSuccess ||
      !hash ||
      !source ||
      !address ||
      !recipient ||
      !destChainKey
    )
      return;
    addTransfer({
      depositTxHash: hash,
      sourceChainKey: source.chainKey,
      destChainKey: Number(destChainKey),
      depositor: address,
      recipient: recipient as `0x${string}`,
      createdAt: Date.now(),
    });
    router.push(`/progress/${hash}`);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- fires once per successful hash
  }, [isSuccess, hash]);

  if (!isConnected) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Connect a wallet</CardTitle>
          <CardDescription>
            Connect a wallet on Base Sepolia or Ethereum Sepolia to bridge.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  if (!source) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Unsupported network</CardTitle>
          <CardDescription>
            Switch to a supported spoke chain (Ethereum Sepolia or Base Sepolia)
            using the network picker in the wallet button above.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  if (destinations.length === 0) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>No destination configured</CardTitle>
          <CardDescription>
            Only {source.chain.name} is configured right now — there is no other
            spoke to bridge to yet.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!source || !destChainKey || !amount || !recipient) return;
    writeContract({
      address: source.bridgeVaultAddress,
      abi: BridgeVaultAbi,
      functionName: "deposit",
      args: [
        zeroAddress,
        parseEther(amount),
        Number(destChainKey),
        recipient as `0x${string}`,
      ],
      value: parseEther(amount),
    });
  }

  const busy = isPending || isConfirming;

  return (
    <Card>
      <CardHeader>
        <CardTitle>Deposit</CardTitle>
        <CardDescription>
          Bridging from {source.chain.name} (native{" "}
          {source.chain.nativeCurrency.symbol}).
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={onSubmit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <Label htmlFor="destination">Destination chain</Label>
            <Select value={destChainKey} onValueChange={setDestChainKeyInput}>
              <SelectTrigger id="destination">
                <SelectValue placeholder="Select destination" />
              </SelectTrigger>
              <SelectContent>
                {destinations.map((d) => (
                  <SelectItem key={d.chainKey} value={String(d.chainKey)}>
                    {d.chain.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col gap-2">
            <Label htmlFor="amount">
              Amount ({source.chain.nativeCurrency.symbol})
            </Label>
            <Input
              id="amount"
              inputMode="decimal"
              placeholder="0.01"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              required
            />
          </div>

          <div className="flex flex-col gap-2">
            <Label htmlFor="recipient">Recipient</Label>
            <Input
              id="recipient"
              placeholder={address}
              value={recipientInput}
              onChange={(e) => setRecipientInput(e.target.value)}
            />
          </div>

          <Button type="submit" disabled={busy}>
            {isPending
              ? "Confirm in wallet..."
              : isConfirming
                ? "Waiting for confirmation..."
                : "Deposit"}
          </Button>

          {error && <p className="text-sm text-destructive">{error.message}</p>}
        </form>
      </CardContent>
    </Card>
  );
}
