// node/src/chain_spec.rs
export const chain_Anvil1_Key = 2;
// Override with ANVIL1_WS_URL for local dev (e.g. ws://localhost:8545 when using default Anvil port)
export const chain_Anvil1_Url = process.env.ANVIL1_WS_URL ?? 'ws://localhost:8141';

export const chain_Anvil2_Key = 4;
export const chain_Anvil2_Id = 31338;
export const chain_Anvil2_Name_Hex = '0x416e76696c32'; // 'Anvil2' in hex
export const chain_Anvil2_Url = 'ws://localhost:8242';

export const chain_Anvil3_Id = 31339;
export const chain_Anvil3_Name = 'Anvil 3';
export const chain_Anvil3_Key = 5;

export const encoding_version_1 = 1;
