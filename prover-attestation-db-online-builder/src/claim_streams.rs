use core::pin::Pin;
//use rand::Rng;
use futures::task::{Context, Poll};
// use std::{sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex}};
// use std::task::Waker;
// use std::thread;
// use std::time::Duration;
use attestation_chain::attestation_checkpoints::AttestationInterval;
use attestation_chain::attestation_checkpoints_for_dev::AttestationCheckpointsForDev;
use ethereum_types::U256;
use prover_primitives::claim::ClaimSerializable;
use utils::json_serializable::JsonSerializable;
use serde::{Deserialize, Serialize};
// pub(crate) struct RandomClaimGenerationStream<'a> {
//     checkpoints: AttestationCheckpointsForDev,
//     cache_dir: Option<&'a str>,
// }

// impl<'a> RandomClaimGenerationStream<'a> {
//     pub fn new(checkpoints: AttestationCheckpointsForDev, cache_dir: Option<&'a str>) -> Self {
//         Self {
//             checkpoints,
//             cache_dir,
//         }
//     }
//     fn checkpoint_range(&mut self) -> Option<core::ops::Range<U256>> {
//         self.checkpoints
//             .poll()
//             .ok()
//             .and_then(|_| {
//                 let checkpoints_tail = self.checkpoints.inner().tail()?;
//                 let checkpoints_head = self.checkpoints.inner().head()?;

//                 let supported_claim_range_start = U256::from(1) + AttestationInterval::interval_for(checkpoints_tail)?
//                     .tail();

//                 Some(supported_claim_range_start..checkpoints_head)
//             })
//     }
// }

// impl futures_util::stream::Stream for RandomClaimGenerationStream<'_> {
//     type Item = ClaimSerializable;

//     fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        
//         Poll::Ready(
//             self
//                 .checkpoint_range()
//                 .and_then(|range| {
//                     let claim_block_number = rand::thread_rng().gen_range(range.start.as_u64()..range.end.as_u64());
//                     let claim_kind = ClaimKind::try_from(rand::thread_rng().gen_range(1..=2)).unwrap();
                    
//                     self
//                         .cache_dir
//                         .map(|dir| <TypedTransaction as FetchFromBlock>::Cache::new(dir, claim_block_number.into()))
//                         .as_ref()
//                         .and_then(|cache| {
//                             let txs = TypedTransaction::fetch_from_cache(cache).unwrap();
//                             let claim_index = if !txs.is_empty() {
//                                 rand::thread_rng().gen_range(0..txs.len())
//                             } else {
//                                 Default::default()
//                             };

//                             let payload_bytes = txs[claim_index].payload_bytes();
//                             let rlp = rlp::Rlp::new(&payload_bytes[..]);

//                             Some(
//                                 ClaimSerializable::from(
//                                     &Claim::try_create( 
//                                         ClaimIdentifier {
//                                             kind: claim_kind,
//                                             block_item_id: BlockItemIdentifier::new(
//                                                 claim_block_number.into(),
//                                                 claim_index  as u64
//                                             ),
//                                         },
//                                         crate::create_sample_query(&txs[claim_index]),
//                                         rlp
//                                     )
//                                     .unwrap()
//                                 )
//                             )        
//                         })
//                 })
//         )
//     }
// }

// struct SeqClaimGenerationKickoffState {
//     kickoff_waker: Option<Waker>,
// }

// pub(crate) struct SeqClaimGenerationStream<'a> {
//     checkpoints: AttestationCheckpointsForDev,
//     cache_dir: Option<&'a str>,
//     curr: Option<U256>,
//     kickoff_state: Arc<Mutex<SeqClaimGenerationKickoffState>>,
//     stream_dropped: Arc<AtomicBool>,
// }

// impl Drop for SeqClaimGenerationStream<'_> {
//     fn drop(&mut self) {
//         self.stream_dropped.store(true, Ordering::Relaxed);
//     }
// }

// impl<'a> SeqClaimGenerationStream<'a> {
//     pub fn new(checkpoints: AttestationCheckpointsForDev, cache_dir: Option<&'a str>) -> Self {
//         let kickoff_state = Arc::new(Mutex::new(SeqClaimGenerationKickoffState {
//             kickoff_waker: None,
//         }));

//         let stream_dropped = Arc::new(AtomicBool::new(false));

//         let kickoff_state_cloned = Arc::clone(&kickoff_state);
//         let stream_dropped_cloned = Arc::clone(&stream_dropped);
//         thread::spawn(move || {
//             while !stream_dropped_cloned.load(Ordering::Relaxed) { 
//                 if let Some(kickoff_waker) = kickoff_state_cloned.lock().unwrap().kickoff_waker.take() {
//                     kickoff_waker.wake()
//                 }
//                 thread::sleep(Duration::from_millis(1000));
//             }
//         });

//         Self {
//             checkpoints,
//             cache_dir,
//             curr: None,
//             kickoff_state,
//             stream_dropped,
//         }
//     }

//     fn checkpoint_range(&mut self) -> Option<core::ops::Range<U256>> {
//         self.checkpoints
//             .poll()
//             .ok()
//             .and_then(|_| {
//                 let checkpoints_tail = self.checkpoints.inner().tail()?;
//                 let checkpoints_head = self.checkpoints.inner().head()?;

//                 let supported_claim_range_start = U256::from(1) + AttestationInterval::interval_for(checkpoints_tail)?
//                     .tail();

//                 Some(supported_claim_range_start..checkpoints_head)
//             })
//     }
// }

// impl futures_util::stream::Stream for SeqClaimGenerationStream<'_> {
//     type Item = ClaimSerializable;

//     fn poll_next(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
//         let checkpoint_range = self.checkpoint_range();

//         if self.curr > checkpoint_range.as_ref().map(|r| r.end) || checkpoint_range.is_none() {
//             println!("claim stream reached checkpoints head, waiting for updating, press ctrl+c to exit");
//             self.kickoff_state.lock().unwrap().kickoff_waker = Some(ctx.waker().clone());
//             return Poll::Pending;
//         }

//         self.kickoff_state.lock().unwrap().kickoff_waker = None;

//         let claim_block_number = self.curr.unwrap_or(
//             checkpoint_range.as_ref().expect("checked earlier").start
//         );
                    
//         let claim = self.cache_dir
//                     .map(|dir| <TypedTransaction as FetchFromBlock>::Cache::new(dir, claim_block_number))
//                     .as_ref()
//                     .and_then(|cache| {
//                         let txs = TypedTransaction::fetch_from_cache(cache).ok()?;
//                         let claim_index = if !txs.is_empty() {
//                             rand::thread_rng().gen_range(0..txs.len())
//                         } else {
//                             Default::default()
//                         };
//                         let payload_bytes = txs[claim_index].payload_bytes();
//                         let rlp = rlp::Rlp::new(&payload_bytes[..]);

//                         Some(
//                             ClaimSerializable::from(
//                                 &Claim::try_create( 
//                                     ClaimIdentifier {
//                                         kind: ClaimKind::Tx,
//                                         block_item_id: BlockItemIdentifier::new(
//                                             claim_block_number.into(),
//                                             claim_index as u64
//                                         ),
//                                     },
//                                     crate::create_sample_query(&txs[claim_index]),
//                                     rlp
//                                 )
//                                 .unwrap()
//                             )
//                         )
//                     });

//         self.curr = self.curr.map(|curr| curr + 1).or(checkpoint_range.map(|r| r.start + 1));
//         Poll::Ready(claim)
//     }
// }

#[derive(Serialize, Deserialize)]
struct ClaimsSerializable(Vec<ClaimSerializable>);

impl JsonSerializable for ClaimsSerializable {}

pub(crate) struct FromJsonClaimGenerationStream {
    claims: ClaimsSerializable,
}

impl FromJsonClaimGenerationStream {
    pub fn try_create(fname: &str) -> anyhow::Result<Self> {
        Ok(Self {
            claims: ClaimsSerializable::try_from_file(fname)?,
        })
    }
}

impl futures_util::stream::Stream for FromJsonClaimGenerationStream {
    type Item = ClaimSerializable;

    fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(
            (!self.claims.0.is_empty())
                .then_some(&mut self.claims.0)
                .and_then(|claims| claims
                    .drain(0..1)
                    .next()
                )
        )
    }
}
