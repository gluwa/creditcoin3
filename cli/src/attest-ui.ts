#!/usr/bin/env node
import { internalSignSendAndWatch } from './lib/tx_fe_for_testing';
import { enableWeb3AndGetListOfAccounts } from './lib/tx_for_fe';
import { callAttest } from './lib/attestation/attest';
import { callChillAttestor } from './lib/attestation/chill';
import { callAttestorClaimRewards } from './lib/attestation/claimRewards';
import { callAttestorRegisterAttestor } from './lib/attestation/registerAttestor';
import { callAttestorSetPayee } from './lib/attestation/setPayee';
import { callAttestorUnregisterAttestor } from './lib/attestation/unregisterAttestor';
import { callAttestorWithdrawUnbonded } from './lib/attestation/withdrawUnbonded';


(window as any).internalSignSendAndWatch = internalSignSendAndWatch;


// //for FE integration
(window as any).callAttest = callAttest;
(window as any).callChillAttestor = callChillAttestor;
(window as any).callAttestorClaimRewards = callAttestorClaimRewards;
(window as any).callAttestorRegisterAttestor = callAttestorRegisterAttestor;
(window as any).callAttestorSetPayee = callAttestorSetPayee;
(window as any).callAttestorUnregisterAttestor = callAttestorUnregisterAttestor;
(window as any).callAttestorWithdrawUnbonded = callAttestorWithdrawUnbonded;

// //use this fn to enable web3 and get list of accounts that are available in the browser with a injected web3 provider
(window as any).enableWeb3AndGetListOfAccounts = enableWeb3AndGetListOfAccounts;



