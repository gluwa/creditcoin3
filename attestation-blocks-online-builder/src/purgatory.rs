use crate::SourceChainBlockIdentifier;
use std::collections::VecDeque;
use std::time::SystemTime;
//use crate::{WithSourceBlockNumber};

pub struct BlockPurgatoryQueue<const PURGATORY_PERIOD_MILLIS: u128> {
    queue: VecDeque<WaitingBlock>,
}

impl<const PURGATORY_PERIOD_MILLIS: u128> BlockPurgatoryQueue<PURGATORY_PERIOD_MILLIS> {
    pub fn new() -> Self {
        Self {
            queue: Default::default(),
        }
    }

    pub fn push(&mut self, block: WaitingBlock) {
        self.queue.push_back(block)
    }

    pub fn expulse(&mut self, max_num_of_blocks_to_expulse: Option<usize>) -> Vec<WaitingBlock> {
        let mut expulsed = Vec::new();
        let now = SystemTime::now();

        while let Some(latest) = self.queue.front() {
            if expulsed.len() >= max_num_of_blocks_to_expulse.unwrap_or(usize::MAX) {
                // apply backpressure
                break;
            }
            if let Ok(elapsed) = now.duration_since(latest.when) {
                if elapsed.as_millis() > self.period() {
                    expulsed.push(self.queue.pop_front().expect("checked for Some"))
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        expulsed
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn period(&self) -> u128 {
        PURGATORY_PERIOD_MILLIS
    }
}

pub(crate) struct WaitingBlock {
    pub block: SourceChainBlockIdentifier,
    pub when: SystemTime,
}

impl From<SourceChainBlockIdentifier> for WaitingBlock {
    fn from(block: SourceChainBlockIdentifier) -> Self {
        Self {
            block,
            when: SystemTime::now(),
        }
    }
}
