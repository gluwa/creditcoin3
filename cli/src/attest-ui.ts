#!/usr/bin/env node
import { balanceAction } from './commands/balance_clean';
import { internalSignSendAndWatch } from './lib/tx_fe';
import { callTransferAdvanced } from './lib/tx_fe_final';
import { callAttest } from './lib/attestation/attest';
import { callChillAttestor } from './lib/attestation/chill';
import { callAttestorClaimRewards } from './lib/attestation/claimRewards';
import { callAttestorRegisterAttestor } from './lib/attestation/registerAttestor';
import { callAttestorSetPayee } from './lib/attestation/setPayee';
import { callAttestorUnregisterAttestor } from './lib/attestation/unregisterAttestor';
import { callAttestorWithdrawUnbonded } from './lib/attestation/withdrawUnbonded';


console.log('balanceAction:', "bind to window");
(window as any).balanceAction = balanceAction;

(window as any).internalSignSendAndWatch = internalSignSendAndWatch;
//for testing purposes
(window as any).callTransferAdvanced = callTransferAdvanced;

(window as any).callAttest = callAttest;
(window as any).callChillAttestor = callChillAttestor;
(window as any).callAttestorClaimRewards = callAttestorClaimRewards;
(window as any).callAttestorRegisterAttestor = callAttestorRegisterAttestor;
(window as any).callAttestorSetPayee = callAttestorSetPayee;
(window as any).callAttestorUnregisterAttestor = callAttestorUnregisterAttestor;
(window as any).callAttestorWithdrawUnbonded = callAttestorWithdrawUnbonded;
