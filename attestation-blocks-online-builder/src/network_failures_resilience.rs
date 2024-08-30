use crate::AsyncCallbackWithArg;
use attestation_chain::block::Block;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use tokio::sync::mpsc::UnboundedReceiver;
use utils::Felt;

#[derive(Debug)]
pub(crate) struct ContinuityHandle {
    block_number: u64,
    root: Felt,
}
impl ContinuityHandle {
    pub fn new(block_number: u64, root: Felt) -> Self {
        Self { block_number, root }
    }

    pub fn block_number(&self) -> u64 {
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
    on_late_block_dropped: Option<AsyncCallbackWithArg<u64, ()>>,
) {
    let mut resiliency_ordering_queue = BinaryHeap::<ContinuityHandle>::new();
    let mut prev_top_opt = None;
    let mut first_block = None;

    loop {
        tokio::select! {
            continuity_handle = rx.recv() => {
                match continuity_handle {
                    Some(continuity_handle) => {
                        let ContinuityHandle { block_number, root } = continuity_handle;

                        match first_block {
                            // the case of the very first block
                            None => {
                                prev_top_opt = Some(block_number);
                                first_block = Some(block_number);

                                if let Some(ref cb) = on_block_ready {
                                    let block = Block::new(block_number, root);
                                    cb(block).await;
                                };
                            },
                            // check if the block is late and is to be dropped
                            Some(first_block) if block_number <= first_block => {
//                                println!("dropped {}, first_block: {:?}", block_number, first_block);
                                if let Some(ref cb) = on_late_block_dropped {
                                    cb(block_number).await;
                                };
                            },

                            _ => {
                                let mut compare_to = prev_top_opt.unwrap() + 1;
                                resiliency_ordering_queue.push(continuity_handle);
                                // set free all the blocks that chain with each other
                                while Some(compare_to) == resiliency_ordering_queue.peek().map(|b| b.block_number) {

                                    let ContinuityHandle {
                                        block_number,
                                        root,
                                    } = resiliency_ordering_queue.pop().expect("checked for Some in peek()");

                                    prev_top_opt = Some(block_number);
                                    compare_to += 1;

                                    if let Some(ref cb) = on_block_ready {
                                        let block = Block::new(block_number, root);
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
