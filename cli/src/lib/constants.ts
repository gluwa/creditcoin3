import { BN, nToBigInt } from '@polkadot/util';

export const MICROUNITS_PER_CTC = new BN((1_000_000_000_000_000_000).toString());
// note: fees are of type BigInt
export const POINT_01_CTC = nToBigInt(MICROUNITS_PER_CTC.div(new BN(100)));
