import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ethers } from "ethers";
import {
  loadConfig,
  saveOverrides,
  loadOverrides,
  isConfigured,
  type BridgeConfig,
  type ChainConfig,
} from "./config";
import { DEST_BRIDGE_ABI, CC_BRIDGE_ABI, ERC20_ABI } from "./abi";
import {
  connectWallet,
  fetchProof,
  readProvider,
  signerFor,
  tokenBalance,
  waitForFinality,
} from "./chains";

// Destination (Sepolia) finality delay before proof-gen can attest a locked-deposit block.
// Sepolia uses EvmFinalized (~2 epochs ≈ 64 blocks).
const DEST_FINALITY_BLOCKS = 64;
// CC→dest: attestors won't observe an Outbox event until it is this many Creditcoin blocks below
// the tip (attestor `blockConfirmationDepth` on usc-dev). At ~5s/block that's ~160s before signing
// even begins; keep this in sync with the deployed attestorset value.
const SRC_CONFIRMATION_BLOCKS = 32;

// A locked deposit awaiting its claim — persisted so a failed/interrupted claim can be resumed
// (locking succeeded on the destination but the proof/claim on Creditcoin didn't finish).
const PENDING_KEY = "usc-bridge-pending-claim";
interface PendingClaim {
  txHash: string;
  lockBlock: number;
  amount: string; // human units, for display + re-toast
}
const loadPending = (): PendingClaim | null => {
  try {
    return JSON.parse(localStorage.getItem(PENDING_KEY) || "null");
  } catch {
    return null;
  }
};
const savePending = (p: PendingClaim | null) =>
  p ? localStorage.setItem(PENDING_KEY, JSON.stringify(p)) : localStorage.removeItem(PENDING_KEY);

type Direction = "ccToDest" | "destToCc";
type StepStatus = "pending" | "active" | "done" | "error";
interface Step {
  label: string;
  status: StepStatus;
}
interface Toast {
  kind: "success" | "error" | "info";
  title: string;
  body?: string;
}

const fmt = (wei: bigint) => Number(ethers.formatUnits(wei, 18)).toLocaleString(undefined, {
  maximumFractionDigits: 4,
});

export function App() {
  const [cfg, setCfg] = useState<BridgeConfig | null>(null);
  const [account, setAccount] = useState<string | null>(null);
  const [ccBal, setCcBal] = useState<bigint>(0n);
  const [destBal, setDestBal] = useState<bigint>(0n);
  const [ccSym, setCcSym] = useState("token");
  const [destSym, setDestSym] = useState("token");
  const [direction, setDirection] = useState<Direction>("ccToDest");
  const [amount, setAmount] = useState("10");
  const [steps, setSteps] = useState<Step[]>([]);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<Toast | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pending, setPending] = useState<PendingClaim | null>(loadPending);
  const [locks, setLocks] = useState<
    { txHash: string; block: number; amount: bigint; nonce: bigint; claimed: boolean }[]
  >([]);
  const [recoverTx, setRecoverTx] = useState("");

  // ETA / countdown for long waiting steps.
  const [waitEta, setWaitEta] = useState(0);
  const [waitStart, setWaitStart] = useState(0);
  const [nowTick, setNowTick] = useState(Date.now());

  useEffect(() => {
    loadConfig().then(setCfg).catch((e) => setToast({ kind: "error", title: "Config error", body: String(e) }));
  }, []);

  useEffect(() => {
    if (!cfg || !isConfigured(cfg)) setSettingsOpen((o) => o || (cfg ? !isConfigured(cfg) : false));
  }, [cfg]);

  // React to MetaMask account changes.
  useEffect(() => {
    if (!window.ethereum) return;
    const onAccts = (a: string[]) => setAccount(a[0] ? ethers.getAddress(a[0]) : null);
    window.ethereum.on?.("accountsChanged", onAccts);
    return () => window.ethereum.removeListener?.("accountsChanged", onAccts);
  }, []);

  const refreshBalances = useCallback(async () => {
    if (!cfg || !account || !isConfigured(cfg)) return;
    try {
      const [cc, an] = await Promise.all([
        tokenBalance(cfg.chains.creditcoin, account),
        tokenBalance(cfg.chains.dest, account),
      ]);
      setCcBal(cc);
      setDestBal(an);
    } catch {
      /* transient RPC hiccup — keep last values */
    }
  }, [cfg, account]);

  // Auto-refresh balances every 5s.
  useEffect(() => {
    refreshBalances();
    const id = setInterval(refreshBalances, 5000);
    return () => clearInterval(id);
  }, [refreshBalances]);

  // Read each token's on-chain symbol once configured (works for wCTC, USD, or any ERC20).
  useEffect(() => {
    if (!cfg || !isConfigured(cfg)) return;
    const read = async (chain: ChainConfig, set: (s: string) => void) => {
      try {
        const t = new ethers.Contract(chain.token, ERC20_ABI, readProvider(chain));
        set(await t.symbol());
      } catch {
        /* leave default */
      }
    };
    read(cfg.chains.creditcoin, setCcSym);
    read(cfg.chains.dest, setDestSym);
  }, [cfg]);

  // 1s tick drives the countdown while a wait is active.
  useEffect(() => {
    if (!waitEta) return;
    const id = setInterval(() => setNowTick(Date.now()), 500);
    return () => clearInterval(id);
  }, [waitEta]);

  const remaining = waitEta ? Math.max(0, waitEta - Math.floor((nowTick - waitStart) / 1000)) : 0;
  const waitPct = waitEta ? Math.min(100, ((waitEta - remaining) / waitEta) * 100) : 0;

  const setStep = (i: number, status: StepStatus) =>
    setSteps((s) => s.map((st, idx) => (idx === i ? { ...st, status } : st)));

  const startWait = (etaSec: number) => {
    setWaitStart(Date.now());
    setNowTick(Date.now());
    setWaitEta(etaSec);
  };
  const endWait = () => setWaitEta(0);

  const onConnect = async () => {
    try {
      setAccount(await connectWallet());
    } catch (e: any) {
      setToast({ kind: "error", title: "Connect failed", body: e.message });
    }
  };

  async function ensureAllowance(chain: ChainConfig, owner: string, spender: string, amt: bigint, stepIdx: number) {
    const signer = await signerFor(chain);
    const token = new ethers.Contract(chain.token, ERC20_ABI, signer);
    const allowance: bigint = await token.allowance(owner, spender);
    if (allowance >= amt) {
      setStep(stepIdx, "done");
      return;
    }
    const tx = await token.approve(spender, amt);
    await tx.wait();
    setStep(stepIdx, "done");
  }

  async function bridgeCcToDest(amt: bigint) {
    if (!cfg || !account) return;
    const cc = cfg.chains.creditcoin;
    const dest = cfg.chains.dest;
    setSteps([
      { label: `Approve ${ccSym} token`, status: "active" },
      { label: `Withdraw on ${cc.name}`, status: "pending" },
      { label: `Attestors observing ${cc.name}`, status: "pending" },
      { label: "Attestors vote · relayer delivers", status: "pending" },
    ]);

    await ensureAllowance(cc, account, cc.bridge, amt, 0);

    setStep(1, "active");
    const signer = await signerFor(cc);
    const ccBridge = new ethers.Contract(cc.bridge, CC_BRIDGE_ABI, signer);
    const before = await tokenBalance(dest, account);
    const tx = await ccBridge.withdraw(amt, account);
    const rcpt = await tx.wait();
    const withdrawBlock: number = Number(rcpt.blockNumber);
    setStep(1, "done");

    // Attestors gate on `blockConfirmationDepth` (32) Creditcoin blocks below the tip before they
    // sign, so proceeding to the delivery poll before then just burns the timeout. Show the live
    // block countdown, same UX as the reverse-direction finality wait.
    setStep(2, "active");
    await waitForFinality(cc, withdrawBlock, SRC_CONFIRMATION_BLOCKS, (rem) =>
      setSteps((s) =>
        s.map((st, idx) =>
          idx === 2 ? { ...st, label: `Attestors observing ${cc.name} — ${rem} blocks left` } : st,
        ),
      ),
    );
    setStep(2, "done");

    // Confirmation reached; now signing + gossip + ⌊2N/3⌋+1 aggregation + the Sepolia delivery tx.
    // That pipeline is fast relative to the 32-block wait, but give it a generous 6-minute poll so
    // a slow relayer round doesn't spuriously fail a bridge that will still land.
    setStep(3, "active");
    await waitForBalanceIncrease(dest, account, before, 120);
    setStep(3, "done");
    await refreshBalances();
    setToast({ kind: "success", title: `Bridged to ${dest.name} ✅`, body: `${fmt(amt)} ${destSym} released on ${dest.name}.` });
  }

  // Sepolia->Creditcoin tail: wait for destination finality, fetch the proof (retries through
  // not-yet-attested + proxy hiccups), and claim on Creditcoin. `i0` is the step index it starts
  // at, so it serves both a fresh lock (i0=2) and a resumed one (i0=0). Clears the pending entry
  // ONLY on success, so any failure stays resumable (locked funds are never stranded).
  async function finalizeClaim(lockTxHash: string, lockBlock: number, amt: bigint, i0: number) {
    if (!cfg || !account) return;
    const cc = cfg.chains.creditcoin;
    const dest = cfg.chains.dest;

    setStep(i0, "active");
    await waitForFinality(dest, lockBlock, DEST_FINALITY_BLOCKS, (rem) =>
      setSteps((s) =>
        s.map((st, idx) =>
          idx === i0 ? { ...st, label: `Waiting for ${dest.name} finality — ${rem} blocks left` } : st,
        ),
      ),
    );
    setStep(i0, "done");

    setStep(i0 + 1, "active");
    startWait(180);
    const proof = await fetchProof(cfg.proofGenPath, cfg.destChainKey, lockTxHash, () => {});
    endWait();
    setStep(i0 + 1, "done");

    setStep(i0 + 2, "active");
    const ccBefore = await tokenBalance(cc, account);
    const ccSigner = await signerFor(cc);
    const ccBridge = new ethers.Contract(cc.bridge, CC_BRIDGE_ABI, ccSigner);
    const claimTx = await ccBridge.claim(
      BigInt(proof.headerNumber),
      proof.txBytes,
      { root: proof.merkleProof.root, siblings: proof.merkleProof.siblings },
      { lowerEndpointDigest: proof.continuityProof.lowerEndpointDigest, roots: proof.continuityProof.roots },
    );
    await claimTx.wait();
    setStep(i0 + 2, "done");
    await waitForBalanceIncrease(cc, account, ccBefore, 20);
    await refreshBalances();
    savePending(null);
    setPending(null);
    setToast({ kind: "success", title: `Bridged to ${cc.name} ✅`, body: `${fmt(amt)} ${ccSym} released on ${cc.name}.` });
  }

  async function bridgeDestToCc(amt: bigint) {
    if (!cfg || !account) return;
    const dest = cfg.chains.dest;
    setSteps([
      { label: `Approve ${destSym} token`, status: "active" },
      { label: `Lock on ${dest.name}`, status: "pending" },
      { label: `Wait for ${dest.name} finality`, status: "pending" },
      { label: "Generate USC proof (await attestation)", status: "pending" },
      { label: `Claim on ${cfg.chains.creditcoin.name}`, status: "pending" },
    ]);

    await ensureAllowance(dest, account, dest.bridge, amt, 0);

    setStep(1, "active");
    const destSigner = await signerFor(dest);
    const destBridge = new ethers.Contract(dest.bridge, DEST_BRIDGE_ABI, destSigner);
    const lockTx = await destBridge.lock(amt, account);
    const lockRcpt = await lockTx.wait();
    setStep(1, "done");

    // Persist BEFORE proof/claim so an interrupted claim (proof-gen down, page reload, revert) is
    // resumable — the tokens are locked, we just need to finish the claim later.
    const p: PendingClaim = {
      txHash: lockRcpt.hash,
      lockBlock: lockRcpt.blockNumber,
      amount: ethers.formatUnits(amt, 18),
    };
    savePending(p);
    setPending(p);

    await finalizeClaim(lockRcpt.hash, lockRcpt.blockNumber, amt, 2);
  }

  // Resume a locked-but-unclaimed deposit (from the pending banner) — runs finality/proof/claim only.
  // Run finality→proof→claim for an already-locked deposit (resume banner, deposit list, or manual
  // tx-hash recovery). Idempotent on-chain: proof-gen re-serves, and CcBridge.claim dedups by
  // (recipient,amount,nonce) — so re-running a claim is always safe.
  const claimLock = useCallback(
    async (txHash: string, block: number, amount: bigint) => {
      if (!cfg || !account || busy) return;
      setBusy(true);
      setToast(null);
      setSteps([
        { label: `Wait for ${cfg.chains.dest.name} finality`, status: "active" },
        { label: "Generate USC proof (await attestation)", status: "pending" },
        { label: `Claim on ${cfg.chains.creditcoin.name}`, status: "pending" },
      ]);
      try {
        await finalizeClaim(txHash, block, amount, 0);
        await loadLocks();
      } catch (e: any) {
        setSteps((s) => s.map((st) => (st.status === "active" ? { ...st, status: "error" } : st)));
        setToast({ kind: "error", title: "Claim failed — still recoverable", body: e?.shortMessage ?? e?.message ?? String(e) });
      } finally {
        endWait();
        setBusy(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [cfg, account, busy],
  );

  const resumeClaim = () =>
    pending && claimLock(pending.txHash, pending.lockBlock, ethers.parseUnits(pending.amount || "0", 18));

  // Manual recovery: given a destination lock tx hash, read its Locked event and claim it.
  const recoverByTxHash = async () => {
    if (!cfg || !recoverTx.trim()) return;
    try {
      const dest = cfg.chains.dest;
      const rcpt = await readProvider(dest).getTransactionReceipt(recoverTx.trim());
      if (!rcpt) return setToast({ kind: "error", title: "Tx not found on " + dest.name });
      const iface = new ethers.Interface(DEST_BRIDGE_ABI);
      const locked = rcpt.logs
        .map((l) => { try { return iface.parseLog(l); } catch { return null; } })
        .find((p) => p?.name === "Locked");
      if (!locked) return setToast({ kind: "error", title: "No Locked event in that tx" });
      setRecoverTx("");
      await claimLock(rcpt.hash, rcpt.blockNumber, locked.args.amount as bigint);
    } catch (e: any) {
      setToast({ kind: "error", title: "Recover failed", body: e?.shortMessage ?? e?.message ?? String(e) });
    }
  };

  // List the connected account's recent locks on the destination bridge + their claimed status.
  const loadLocks = useCallback(async () => {
    if (!cfg || !account || !isConfigured(cfg)) return;
    try {
      const dest = cfg.chains.dest;
      const cc = cfg.chains.creditcoin;
      const dp = readProvider(dest);
      const bridge = new ethers.Contract(dest.bridge, DEST_BRIDGE_ABI, dp);
      const ccBridge = new ethers.Contract(cc.bridge, CC_BRIDGE_ABI, readProvider(cc));
      const head = await dp.getBlockNumber();
      const events = await bridge.queryFilter(bridge.filters.Locked(account), Math.max(0, head - 9000), head);
      const coder = ethers.AbiCoder.defaultAbiCoder();
      const rows = await Promise.all(
        events.map(async (ev: any) => {
          const { ccRecipient, amount, nonce } = ev.args;
          const key = ethers.keccak256(coder.encode(["address", "uint256", "uint256"], [ccRecipient, amount, nonce]));
          let claimed = false;
          try { claimed = await ccBridge.claimed(key); } catch { /* ignore */ }
          return { txHash: ev.transactionHash, block: ev.blockNumber, amount, nonce, claimed };
        }),
      );
      setLocks(rows.reverse());
    } catch {
      /* transient — leave last list */
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cfg, account]);

  useEffect(() => {
    loadLocks();
  }, [loadLocks]);

  async function waitForBalanceIncrease(chain: ChainConfig, account: string, before: bigint, timeoutMs60: number) {
    const provider = readProvider(chain);
    const token = new ethers.Contract(chain.token, ERC20_ABI, provider);
    for (let i = 0; i < timeoutMs60; i++) {
      const cur: bigint = await token.balanceOf(account);
      if (cur > before) return cur;
      await new Promise((r) => setTimeout(r, 3000));
    }
    throw new Error("timed out waiting for the destination release");
  }

  const onBridge = async () => {
    if (!cfg || !account) return;
    let amt: bigint;
    try {
      amt = ethers.parseUnits(amount || "0", 18);
    } catch {
      return setToast({ kind: "error", title: "Invalid amount" });
    }
    if (amt <= 0n) return setToast({ kind: "error", title: "Amount must be > 0" });
    const srcBal = direction === "ccToDest" ? ccBal : destBal;
    if (amt > srcBal) return setToast({ kind: "error", title: "Insufficient balance" });

    setBusy(true);
    setToast(null);
    try {
      if (direction === "ccToDest") await bridgeCcToDest(amt);
      else await bridgeDestToCc(amt);
    } catch (e: any) {
      setSteps((s) => s.map((st) => (st.status === "active" ? { ...st, status: "error" } : st)));
      setToast({ kind: "error", title: "Bridge failed", body: e?.shortMessage ?? e?.message ?? String(e) });
    } finally {
      endWait();
      setBusy(false);
    }
  };

  const src = direction === "ccToDest" ? cfg?.chains.creditcoin : cfg?.chains.dest;
  const dst = direction === "ccToDest" ? cfg?.chains.dest : cfg?.chains.creditcoin;
  const srcBal = direction === "ccToDest" ? ccBal : destBal;
  const configured = cfg ? isConfigured(cfg) : false;
  // Display names come from the config so the UI reflects the target chains (e.g. Sepolia).
  const ccName = cfg?.chains.creditcoin.name ?? "Creditcoin";
  const dstName = cfg?.chains.dest.name ?? "Sepolia";

  return (
    <div className="page">
      <header className="topbar">
        <div className="brand">
          <img className="logo" src="/creditcoin-icon.png" alt="Creditcoin" />
          <div>
            <h1>Creditcoin USC</h1>
            <p>Bridge · {ccName} ⇄ {dstName}</p>
          </div>
        </div>
        <div className="topbar-actions">
          <button className="ghost" onClick={() => setSettingsOpen(true)}>⚙ Settings</button>
          {account ? (
            <span className="pill account">{account.slice(0, 6)}…{account.slice(-4)}</span>
          ) : (
            <button className="primary sm" onClick={onConnect}>Connect MetaMask</button>
          )}
        </div>
      </header>

      {!configured && (
        <div className="banner">
          ⚠ Contract addresses not set. Open <b>Settings</b> and paste the bridge addresses from the
          deploy output (or run <code>npm run gen-config</code>).
        </div>
      )}

      {pending && (
        <div className="banner">
          ⏳ You have a locked {dstName} deposit ({fmt(ethers.parseUnits(pending.amount || "0", 18))} {destSym})
          awaiting its claim on {ccName} — tx <code>{pending.txHash.slice(0, 10)}…{pending.txHash.slice(-6)}</code>.
          <button className="primary sm" style={{ marginLeft: 8 }} disabled={!account || busy} onClick={resumeClaim}>
            Resume claim
          </button>
          <button className="ghost sm" style={{ marginLeft: 6 }} disabled={busy}
            onClick={() => { savePending(null); setPending(null); }}>
            Dismiss
          </button>
        </div>
      )}

      <main className="grid">
        <section className="balances">
          <BalanceCard label={ccName} sub={ccSym} bal={ccBal} active={direction === "ccToDest"} />
          <BalanceCard label={dstName} sub={destSym} bal={destBal} active={direction === "destToCc"} />
        </section>

        <section className="card bridge">
          <div className="seg">
            <button
              className={direction === "ccToDest" ? "on" : ""}
              onClick={() => setDirection("ccToDest")}
              disabled={busy}
            >
              {ccName} → {dstName} <em>votes</em>
            </button>
            <button
              className={direction === "destToCc" ? "on" : ""}
              onClick={() => setDirection("destToCc")}
              disabled={busy}
            >
              {dstName} → {ccName} <em>proof</em>
            </button>
          </div>

          <div className="route">
            <span className="chip">{src?.name ?? "—"}</span>
            <span className="arrow">⟶</span>
            <span className="chip">{dst?.name ?? "—"}</span>
          </div>

          <label className="amount">
            <div className="amount-row">
              <input
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                inputMode="decimal"
                placeholder="0.0"
                disabled={busy}
              />
              <button className="max" disabled={busy} onClick={() => setAmount(ethers.formatUnits(srcBal, 18))}>
                MAX
              </button>
            </div>
            <div className="hint">Balance: {fmt(srcBal)}</div>
          </label>

          <button className="primary bridge-btn" disabled={!account || !configured || busy} onClick={onBridge}>
            {busy ? "Bridging…" : !account ? "Connect wallet first" : "Bridge"}
          </button>

          {steps.length > 0 && (
            <div className="progress">
              {waitEta > 0 && (
                <div className="eta">
                  <div className="eta-bar"><div className="eta-fill" style={{ width: `${waitPct}%` }} /></div>
                  <span>~{remaining}s remaining</span>
                </div>
              )}
              <ol className="steps">
                {steps.map((s, i) => (
                  <li key={i} className={s.status}>
                    <span className="dot" />
                    {s.label}
                  </li>
                ))}
              </ol>
            </div>
          )}
        </section>

        <section className="card bridge">
          <div className="route" style={{ justifyContent: "space-between" }}>
            <span className="chip">Your {dstName} → {ccName} deposits</span>
            <button className="ghost sm" disabled={busy} onClick={loadLocks}>↻ Refresh</button>
          </div>

          {locks.length === 0 ? (
            <p style={{ opacity: 0.6, fontSize: "0.9em" }}>
              No recent locks found for this account on {dstName}. If you locked earlier, paste the lock tx below.
            </p>
          ) : (
            <ol className="steps">
              {locks.map((l) => (
                <li key={l.txHash} className={l.claimed ? "done" : "pending"} style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
                  <span>
                    <span className="dot" />
                    {fmt(l.amount)} {destSym} · <code>{l.txHash.slice(0, 8)}…{l.txHash.slice(-4)}</code>
                  </span>
                  {l.claimed ? (
                    <span className="pill">claimed ✓</span>
                  ) : (
                    <button className="primary sm" disabled={!account || busy}
                      onClick={() => claimLock(l.txHash, l.block, l.amount)}>
                      Claim on {ccName}
                    </button>
                  )}
                </li>
              ))}
            </ol>
          )}

          <label className="amount recover" style={{ marginTop: 10 }}>
            <div className="amount-row">
              <input
                placeholder="Recover by lock tx hash (0x…)"
                value={recoverTx}
                onChange={(e) => setRecoverTx(e.target.value)}
                disabled={busy}
              />
              <button className="max" disabled={busy || !recoverTx.trim()} onClick={recoverByTxHash}>
                Claim
              </button>
            </div>
          </label>
        </section>
      </main>

      {toast && <ToastView toast={toast} onClose={() => setToast(null)} />}
      {settingsOpen && cfg && (
        <Settings
          cfg={cfg}
          onClose={() => setSettingsOpen(false)}
          onSave={async () => {
            setSettingsOpen(false);
            setCfg(await loadConfig());
          }}
        />
      )}
    </div>
  );
}

function BalanceCard({ label, sub, bal, active }: { label: string; sub: string; bal: bigint; active: boolean }) {
  return (
    <div className={"card balance" + (active ? " active" : "")}>
      <div className="balance-head">
        <span>{label}</span>
        <span className="live">● live</span>
      </div>
      <div className="balance-amt">{fmt(bal)}</div>
      <div className="balance-sub">{sub}</div>
    </div>
  );
}

function ToastView({ toast, onClose }: { toast: Toast; onClose: () => void }) {
  useEffect(() => {
    if (toast.kind === "success") {
      const id = setTimeout(onClose, 6000);
      return () => clearTimeout(id);
    }
  }, [toast, onClose]);
  return (
    <div className={"toast " + toast.kind}>
      <div className="toast-icon">{toast.kind === "success" ? "✅" : toast.kind === "error" ? "⚠" : "ℹ"}</div>
      <div className="toast-body">
        <strong>{toast.title}</strong>
        {toast.body && <p>{toast.body}</p>}
      </div>
      <button className="toast-x" onClick={onClose}>×</button>
    </div>
  );
}

function Settings({ cfg, onClose, onSave }: { cfg: BridgeConfig; onClose: () => void; onSave: () => void }) {
  const o = useMemo(() => loadOverrides(), []);
  const form = useRef({
    ccToken: o.ccToken ?? cfg.chains.creditcoin.token,
    ccBridge: o.ccBridge ?? cfg.chains.creditcoin.bridge,
    destToken: o.destToken ?? cfg.chains.dest.token,
    destBridge: o.destBridge ?? cfg.chains.dest.bridge,
  });
  const [, force] = useState(0);
  const set = (k: keyof typeof form.current) => (e: any) => {
    form.current[k] = e.target.value.trim();
    force((n) => n + 1);
  };
  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Bridge addresses</h2>
        <p className="muted">From the deploy output (or <code>npm run gen-config</code>). Saved locally.</p>
        {(["ccToken", "ccBridge", "destToken", "destBridge"] as const).map((k) => (
          <label key={k} className="field">
            <span>{k}</span>
            <input value={form.current[k]} onChange={set(k)} placeholder="0x…" spellCheck={false} />
          </label>
        ))}
        <div className="modal-actions">
          <button className="ghost" onClick={onClose}>Cancel</button>
          <button className="primary" onClick={() => { saveOverrides({ ...form.current }); onSave(); }}>Save</button>
        </div>
      </div>
    </div>
  );
}
