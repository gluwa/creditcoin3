import { execFileSync } from "node:child_process";

export function isValidPrivateKey(key: string | undefined): key is string {
  return !!key && key.startsWith("0x") && key.length === 66;
}

export function isValidContractAddress(
  address: string | undefined,
): address is string {
  return !!address && address.startsWith("0x") && address.length === 42;
}

export function isValidBytes32(value: string | undefined): value is string {
  return !!value && /^0x[0-9a-fA-F]{64}$/.test(value);
}

export function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing ${name}`);
  }
  return value;
}

export function parseArg(name: string, short?: string): string | undefined {
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === name || (short && args[i] === short)) {
      return args[i + 1];
    }
    if (args[i].startsWith(`${name}=`)) {
      return args[i].slice(name.length + 1);
    }
  }
  return undefined;
}

export function getPayeeAddress(dir: string): string {
  const privateKey = requireEnv("CREDITCOIN_CHAIN_PRIVATE_KEY");
  const output = runCommand(
    "cast",
    ["wallet", "address", "--private-key", privateKey],
    dir,
  );
  return output.trim();
}

export function getDestinationAddress(dir: string): string {
  const privateKey = requireEnv("DESTINATION_CHAIN_PRIVATE_KEY");
  const output = runCommand(
    "cast",
    ["wallet", "address", "--private-key", privateKey],
    dir,
  );
  return output.trim();
}

export function runCommand(cmd: string, args: string[], cwd: string): string {
  try {
    const output = execFileSync(cmd, args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return output;
  } catch (err: any) {
    const stdout = err?.stdout ? String(err.stdout) : "";
    const stderr = err?.stderr ? String(err.stderr) : "";
    const combined = [stdout, stderr].filter(Boolean).join("\n");
    throw new Error(`Command failed: ${cmd} ${args.join(" ")}\n${combined}`);
  }
}
