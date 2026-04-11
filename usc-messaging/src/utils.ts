export function isValidPrivateKey(key: string | undefined): key is string {
  return !!key && key.startsWith("0x") && key.length === 66;
}

export function isValidContractAddress(
  address: string | undefined,
): address is string {
  return !!address && address.startsWith("0x") && address.length === 42;
}
