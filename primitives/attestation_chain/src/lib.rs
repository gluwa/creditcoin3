mod dense_checkpoints;

pub mod attestation_checkpoints;
pub mod attestation_checkpoints_for_dev;
pub mod attestation_fragment;
pub mod block;
pub mod utils;

use crate::attestation_checkpoints::AttestationInterval;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AttestationChainParams {
    genesis: u64,
    interval: usize,
}

impl AttestationChainParams {
    pub fn new(genesis: u64, interval: usize) -> Self {
        Self { genesis, interval }
    }

    pub fn interval(&self) -> usize {
        self.interval
    }

    pub fn fragment_size(&self) -> usize {
        self.interval + 1
    }

    pub fn genesis(&self) -> u64 {
        self.genesis
    }

    pub fn index_for(&self, b: u64) -> Option<u64> {
        b.checked_sub(self.genesis)
    }

    pub fn checkpoint_number_for(&self, b: u64) -> Option<u64> {
        let interval = self.interval as u64;

        b.checked_sub(self.genesis)
            .map(|d| self.genesis + interval * (d / interval + u64::from(b % interval != 0u64)))
    }

    pub fn interval_for(&self, b: u64) -> Option<AttestationInterval> {
        if b == self.genesis {
            return None;
        }
        self.checkpoint_number_for(b - u64::from(self.is_aligned(b)))
            .and_then(|head| {
                head.checked_sub(self.interval as u64)
                    .map(|tail| AttestationInterval(tail, head))
            })
    }

    pub fn index_in_interval_for(&self, b: u64) -> Option<usize> {
        self.index_for(b)
            .map(|delta| (delta % self.interval as u64) as usize)
    }

    pub fn is_aligned(&self, b: u64) -> bool {
        self.index_in_interval_for(b) == Some(0)
    }
}

const ETH_CHECKPOINT_INTERVAL_DEV: usize = 4;
const ETH_ATTESTATION_GENESIS_DEV: u64 = 0;
//const ETH_ATTESTATION_GENESIS_DEV: u64 = 19504000;

pub const ETH_ATTESTATION_CHAIN_PARAMS_DEV: AttestationChainParams = AttestationChainParams {
    genesis: ETH_ATTESTATION_GENESIS_DEV,
    interval: ETH_CHECKPOINT_INTERVAL_DEV,
};
