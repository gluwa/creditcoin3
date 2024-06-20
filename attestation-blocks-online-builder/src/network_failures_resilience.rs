use crate::AsyncCallbackWithArg;
use attestation_chain::block::Block;
use ethereum_types::U256;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use tokio::sync::mpsc::UnboundedReceiver;
use utils::Felt;

#[derive(Debug)]
pub(crate) struct ContinuityHandle {
    block_number: U256,
    roots: (Felt, Felt),
}
impl ContinuityHandle {
    pub fn new(block_number: U256, roots: (Felt, Felt)) -> Self {
        Self {
            block_number,
            roots,
        }
    }

    pub fn block_number(&self) -> U256 {
        self.block_number
    }
}
impl PartialOrd for ContinuityHandle {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ContinuityHandle {
    fn cmp(&self, other: &Self) -> Ordering {
        self.block_number.cmp(&other.block_number).reverse()
    }
}
impl PartialEq for ContinuityHandle {
    fn eq(&self, other: &Self) -> bool {
        self.block_number == other.block_number
    }
}
impl Eq for ContinuityHandle {}

pub(crate) async fn resiliency_queue_event_loop(
    mut rx: UnboundedReceiver<ContinuityHandle>,
    mut reset_receiver: UnboundedReceiver<()>,

    on_block_ready: Option<AsyncCallbackWithArg<Block, ()>>,
) {
    let mut resiliency_ordering_queue = BinaryHeap::<ContinuityHandle>::new();
    let mut prev_top_opt = None;

    loop {
        tokio::select! {
            continuity_handle = rx.recv() => {
                match continuity_handle {
                    Some(continuity_handle) => {
                        let ContinuityHandle { block_number, roots } = continuity_handle;

                        match prev_top_opt {
                            // the case of the very first block
                            None => {
                                prev_top_opt = Some(block_number);

                                if let Some(ref cb) = on_block_ready {
                                    let block = Block::new(block_number, roots.0, roots.1);
                                    cb(block).await;
                                };
                            },
                            // check if the block is late and is to be dropped
                            Some(prev_top) if continuity_handle.block_number <= prev_top => {
                                println!("dropped");
                                // if let Some(ref cb) = on_append_block_to_attestation_chain_outcome {
                                //     cb(Err(AttestationFragmentError::Other(format!("late block: {}", continuity_handle.block_number)))).await;
                                // };
                            },

                            Some(prev_top) => {
                                let mut compare_to = prev_top + 1;
                                resiliency_ordering_queue.push(continuity_handle);
                                // set free all the blocks that chain with each other
                                while Some(compare_to) == resiliency_ordering_queue.peek().map(|b| b.block_number) {

                                    let ContinuityHandle {
                                        block_number,
                                        roots,
                                    } = resiliency_ordering_queue.pop().expect("checked for Some in peek()");

                                    prev_top_opt = Some(block_number);
                                    compare_to += 1.into();

                                    if let Some(ref cb) = on_block_ready {
                                        let block = Block::new(block_number, roots.0, roots.1);
                                        cb(block).await;
                                    };
                                }
                            }
                        }
                    },

                    None => break,
                }
            },

            reset = reset_receiver.recv() => if let Some(()) = reset {
                prev_top_opt = None;
                resiliency_ordering_queue = Default::default();
            },
        }
    }
}
