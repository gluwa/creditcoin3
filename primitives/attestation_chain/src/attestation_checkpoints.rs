use crate::block::Block;
use crate::dense_checkpoints::{DenseCheckpoints, DenseCheckpointsSerializable};
use crate::{ATTESTATION_GENESIS, CHECKPOINT_INTERVAL};
use ethereum_types::U256;
use serde::{Deserialize, Serialize};
use utils::json_serializable::JsonSerializable;
use utils::Felt;

#[derive(Debug, PartialEq)]
pub enum AttestationCheckpointError {
    HeadCheckpointExpected(U256, U256),
    TailCheckpointExpected(U256),
    GenesisReached,
    DiscontinuedCheckpoints,
    PrependToUnstabilized,
    MisalignedStabilizedCheckpoint(U256),
    InvalidAttestationCheckpoint(U256),
    Other(String),
}

#[derive(PartialEq, Debug, Clone, Copy, Default)]
pub struct AttestationCheckpoint {
    block_number: U256,
    digest: Felt,
}

impl AttestationCheckpoint {
    pub fn try_from_block(block_number: U256, digest: Felt) -> Option<Self> {
        Self::index(block_number).map(|_| Self {
            block_number,
            digest,
        })
    }
    pub fn digest(&self) -> &Felt {
        &self.digest
    }
    pub fn n(&self) -> U256 {
        self.block_number
    }
    pub fn index(b: U256) -> Option<U256> {
        b.checked_sub(ATTESTATION_GENESIS)
    }
    pub fn checkpoint_for(b: U256) -> Option<U256> {
        b.checked_sub(ATTESTATION_GENESIS).map(|d| {
            //            CHECKPOINT_INTERVAL as U256 * (d / CHECKPOINT_INTERVAL as U256 + 1)
            ATTESTATION_GENESIS
                + CHECKPOINT_INTERVAL
                    * (d.as_usize() / CHECKPOINT_INTERVAL
                        + usize::from(b % Into::<U256>::into(CHECKPOINT_INTERVAL) != 0.into()))
        })
    }
}

impl<'a> From<&'a Block> for AttestationCheckpoint {
    fn from(block: &Block) -> Self {
        Self {
            block_number: block.n(),
            digest: block.digest(),
        }
    }
}

impl From<StabilizedCheckpoint> for AttestationCheckpoint {
    fn from(stabilized_checkpoint: StabilizedCheckpoint) -> Self {
        stabilized_checkpoint.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationCheckpointSerializable {
    block_number: Option<String>,
    digest: Option<String>,
}

impl From<Option<&AttestationCheckpoint>> for AttestationCheckpointSerializable {
    fn from(cp_opt: Option<&AttestationCheckpoint>) -> Self {
        Self {
            block_number: cp_opt.map(|cp| cp.block_number.to_string()),
            digest: cp_opt.map(|cp| cp.digest.to_string()),
        }
    }
}

impl From<Option<&StabilizedCheckpoint>> for AttestationCheckpointSerializable {
    fn from(scp_opt: Option<&StabilizedCheckpoint>) -> Self {
        Self {
            block_number: scp_opt.map(|scp| scp.0.block_number.to_string()),
            digest: scp_opt.map(|scp| scp.0.digest.to_string()),
        }
    }
}

impl TryFrom<&AttestationCheckpointSerializable> for Option<AttestationCheckpoint> {
    type Error = ();

    fn try_from(cp: &AttestationCheckpointSerializable) -> Result<Self, ()> {
        let block_number = match &cp.block_number {
            None => return Ok(None),
            Some(block_number) => U256::from_dec_str(block_number).map_err(|_| ())?,
            //            Some(block_number) => block_number.parse().map_err(|_| ())?,
        };

        let digest = cp
            .digest
            .as_ref()
            .ok_or(())
            .and_then(|digest| Felt::from_dec_str(digest.as_ref()).map_err(|_| ()))?;

        Ok(Some(AttestationCheckpoint {
            block_number,
            digest,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttestationInterval(U256, U256);

impl AttestationInterval {
    pub fn interval_for(b: U256) -> Option<Self> {
        if b == ATTESTATION_GENESIS {
            return None;
        }
        AttestationCheckpoint::checkpoint_for(b - if Self::is_aligned(b) { 1 } else { 0 }).and_then(
            |head| {
                head.checked_sub(Into::<U256>::into(CHECKPOINT_INTERVAL))
                    .map(|tail| Self(tail, head))
            },
        )
    }
    pub fn index(b: U256) -> Option<usize> {
        b.checked_sub(ATTESTATION_GENESIS)
            .map(|delta| delta.as_usize() % CHECKPOINT_INTERVAL)
    }

    pub fn is_aligned(b: U256) -> bool {
        Self::index(b) == Some(0)
    }

    pub fn tail(&self) -> U256 {
        self.0
    }
    pub fn head(&self) -> U256 {
        self.1
    }
    pub fn prev(&self) -> Option<Self> {
        Self::interval_for(self.tail())
    }
    pub fn next(&self) -> Self {
        Self::interval_for(self.head() + 1).expect("can get next interval")
    }
}

#[derive(Debug, Clone, PartialEq)]
struct StabilizedCheckpoint(AttestationCheckpoint);

impl StabilizedCheckpoint {
    fn index(&self) -> usize {
        self.0
            .n()
            .checked_sub(ATTESTATION_GENESIS)
            .map(|delta| (delta / Into::<U256>::into(CHECKPOINT_INTERVAL)).as_usize())
            .expect("stabilized checkpoint is aligned with respect to genesis")
    }

    fn n_from_index(index: usize) -> U256 {
        Into::<U256>::into(index * CHECKPOINT_INTERVAL) + ATTESTATION_GENESIS
    }

    fn try_next_from(&self, cp: AttestationCheckpoint) -> Result<Self, AttestationCheckpointError> {
        let scp = Self::try_from(cp)?;

        if scp.index() == self.index() + 1 {
            Ok(scp)
        } else {
            Err(AttestationCheckpointError::HeadCheckpointExpected(
                Self::n_from_index(self.index() + 1),
                cp.n(),
            ))
        }
    }

    fn try_prev_from(&self, cp: AttestationCheckpoint) -> Result<Self, AttestationCheckpointError> {
        let scp = Self::try_from(cp)?;
        if scp.index() + 1 == self.index() {
            Ok(scp)
        } else if self.index() == 0 {
            Err(AttestationCheckpointError::GenesisReached)
        } else {
            Err(AttestationCheckpointError::TailCheckpointExpected(
                Self::n_from_index(self.index() - 1),
            ))
        }
    }
}

impl TryFrom<AttestationCheckpoint> for StabilizedCheckpoint {
    type Error = AttestationCheckpointError;

    fn try_from(cp: AttestationCheckpoint) -> Result<Self, AttestationCheckpointError> {
        if AttestationInterval::is_aligned(cp.n()) {
            Ok(Self(cp))
        } else {
            Err(AttestationCheckpointError::MisalignedStabilizedCheckpoint(
                cp.n(),
            ))
        }
    }
}

#[derive(Debug, Default)]
pub struct AttestationCheckpoints {
    stabilized: Vec<Option<StabilizedCheckpoint>>,
    dense_checkpoints: DenseCheckpoints,
    tail: Option<StabilizedCheckpoint>,
}

impl AttestationCheckpoints {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn interval(&self) -> usize {
        CHECKPOINT_INTERVAL
    }

    pub fn tail(&self) -> Option<U256> {
        self.tail.as_ref().map(|tail| tail.0.n())
    }
    pub fn head(&self) -> Option<U256> {
        self.dense_checkpoints.head()
    }
    pub fn stabilized_head(&self) -> Option<U256> {
        self.stabilized
            .last()
            .unwrap_or(&None)
            .as_ref()
            .map(|last| last.0.n())
    }
    pub fn checkpoint_for(&self, b: U256) -> Option<AttestationCheckpoint> {
        let bcp = AttestationCheckpoint::checkpoint_for(b)?;

        let scp = AttestationCheckpoint::try_from_block(bcp, Default::default())
            .and_then(|cp| StabilizedCheckpoint::try_from(cp).ok())?;
        //        println!("bcp: {bcp:?}, block: {b}, index: {}, stabilized len: {}", scp.index(), self.stabilized.len());
        if scp.index() < self.stabilized.len() {
            self.stabilized[scp.index()].as_ref().map(|scp| scp.0)
        } else {
            self.dense_checkpoints.checkpoint_for(b).copied()
        }
    }
    // pub fn checkpoint_block_number_for(&self, b: U256) -> Option<U256> {
    //     self.checkpoint_for().map(AttestationCheckpoint::n)
    // }

    pub fn try_append(
        &mut self,
        cp: AttestationCheckpoint,
    ) -> Result<(), AttestationCheckpointError> {
        let stabilized_cp_opt = self.dense_checkpoints.try_append(cp)?;
        stabilized_cp_opt
            .map(|stabilized_cp| self.try_append_stabilized(stabilized_cp))
            .unwrap_or_else(|| {
                if self.stabilized.is_empty() {
                    if let Some(head) = self.head() {
                        if let Some(future_stabilized) = AttestationCheckpoint::checkpoint_for(head)
                        {
                            let checkpoint = AttestationCheckpoint::try_from_block(
                                future_stabilized,
                                Default::default(),
                            )
                            .expect("checkpoint can be created");

                            let future_stabilized = StabilizedCheckpoint::try_from(checkpoint)?;

                            self.stabilized = vec![None; future_stabilized.index()];
                        }
                    }
                }
                Ok(())
            })
    }

    pub fn try_prepend(
        &mut self,
        cp: AttestationCheckpoint,
    ) -> Result<(), AttestationCheckpointError> {
        //        let tail = self.tail.as_ref().ok_or(AttestationCheckpointError::PrependToUnstabilized)?;
        let scp = match self.tail.as_ref() {
            Some(tail) => StabilizedCheckpoint::try_prev_from(tail, cp),
            // stabilized is empty, still can prepend if there are dense checkpoints
            None if self.head() > Some(cp.n()) => StabilizedCheckpoint::try_from(cp),
            None => Err(AttestationCheckpointError::PrependToUnstabilized),
        }?;
        self.stabilized[scp.index()] = Some(scp.clone());
        self.tail = Some(scp);
        Ok(())
    }

    pub fn any(&self, checkpoint: &AttestationCheckpoint) -> bool {
        let found_stabilized = match StabilizedCheckpoint::try_from(*checkpoint) {
            Ok(scp) => {
                //                    println!("index: {:?}", scp.index());
                //                    println!("stabilized: {:?}", self.stabilized[scp.index()]);
                if scp.index() < self.stabilized.len() {
                    self.stabilized[scp.index()]
                        .as_ref()
                        .map(|scp| &scp.0 == checkpoint)
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        found_stabilized || self.dense_checkpoints.any(checkpoint)
    }
    pub fn verify_claim_continuity(&self, checkpoint_for_claim: &AttestationCheckpoint) -> bool {
        self.any(checkpoint_for_claim)
    }
}

impl AttestationCheckpoints {
    fn try_append_stabilized(
        &mut self,
        cp: AttestationCheckpoint,
    ) -> Result<(), AttestationCheckpointError> {
        let stabilized_head = self.stabilized.last();
        let stabilized_checkpoint = stabilized_head
            .map(|stabilized_head| {
                stabilized_head
                    .as_ref()
                    .map(|stabilized_head| stabilized_head.try_next_from(cp))
                    .unwrap_or_else(|| StabilizedCheckpoint::try_from(cp))
            })
            // stabilized buffer is empty
            .unwrap_or_else(|| {
                let stabilized_checkpoint = StabilizedCheckpoint::try_from(cp)?;
                // create stabilized vector back to genesis
                self.stabilized = vec![None; stabilized_checkpoint.index()];

                Ok(stabilized_checkpoint)
            })?;

        if self.tail.is_none() {
            self.tail = Some(stabilized_checkpoint.clone());
        }
        self.stabilized.push(Some(stabilized_checkpoint));
        Ok(())
    }
}

impl AttestationCheckpoints {
    pub fn try_from_file(fname: &str) -> Result<Self, AttestationCheckpointError> {
        let checkpoints = AttestationCheckpointsSerializable::try_from_file(fname)
            .map_err(|err| AttestationCheckpointError::Other(format!("{err:?}")))?;

        Self::try_from(checkpoints)
    }

    pub fn to_file(&self, fname: &str) -> Result<(), AttestationCheckpointError> {
        AttestationCheckpointsSerializable::from(self)
            .to_file(fname)
            .map_err(|err| AttestationCheckpointError::Other(format!("{err:?}")))
    }
}

impl TryFrom<AttestationCheckpointsSerializable> for AttestationCheckpoints {
    type Error = AttestationCheckpointError;

    fn try_from(checkpoints_json: AttestationCheckpointsSerializable) -> Result<Self, Self::Error> {
        let mut checkpoints = Self::new();

        for cp_res in checkpoints_json
            .checkpoints
            .iter()
            .map(Option::<AttestationCheckpoint>::try_from)
        {
            let cp_opt = cp_res.map_err(|()| {
                AttestationCheckpointError::Other("failed to parse checkpoint".to_owned())
            })?;
            if let Some(cp) = cp_opt {
                checkpoints.try_append_stabilized(cp)?;
            }
        }
        checkpoints.dense_checkpoints = TryFrom::try_from(checkpoints_json.dense_checkpoints)
            .map_err(|()| {
                AttestationCheckpointError::Other("failed to parse dense checkpoints".to_owned())
            })?;
        Ok(checkpoints)
    }
}

impl std::fmt::Display for AttestationCheckpoints {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "[")?;
        for scp in &self.stabilized {
            write!(
                f,
                "{},",
                scp.as_ref()
                    .map(|scp| AttestationCheckpoint::from(scp.clone()).n().to_string())
                    .unwrap_or("_".to_string())
            )?;
        }
        write!(f, "{},", self.dense_checkpoints)?;
        write!(f, "]")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationCheckpointsSerializable {
    checkpoints: Vec<AttestationCheckpointSerializable>,
    dense_checkpoints: DenseCheckpointsSerializable,
    tail: AttestationCheckpointSerializable,
}

impl From<&AttestationCheckpoints> for AttestationCheckpointsSerializable {
    fn from(checkpoints: &AttestationCheckpoints) -> Self {
        Self {
            checkpoints: checkpoints
                .stabilized
                .iter()
                .map(Option::<_>::as_ref)
                .map(From::from)
                .collect(),
            dense_checkpoints: From::from(&checkpoints.dense_checkpoints),
            tail: From::from(checkpoints.tail.as_ref().map(|tail| &tail.0)),
        }
    }
}

impl JsonSerializable for AttestationCheckpointsSerializable {}

#[cfg(test)]
mod tests {
    use crate::attestation_checkpoints::{
        AttestationCheckpoint, AttestationCheckpointError, AttestationCheckpoints,
    };
    use crate::CHECKPOINT_INTERVAL;
    use utils::Felt;

    #[test]
    fn basic_append_test1() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 0u64..8 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }

        for block_number in 8u64..21 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }

    #[test]
    fn basic_append_test2() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 8u64..16 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }

        for block_number in 16u64..29 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        println!("{}", checkpoints);

        // for block_number in 21u64..24 {
        //     checkpoints.try_append(AttestationCheckpoint {
        //         digest: Felt::from(block_number),
        //         block_number
        //     }).unwrap();
        // }
        // println!("{}", checkpoints);

        // let block_number = 24;
        // checkpoints.try_append(AttestationCheckpoint {
        //     digest: Felt::from(block_number),
        //     block_number
        // }).unwrap();
        // println!("{}", checkpoints);

        // assert!(checkpoints.verify_claim_continuity(&AttestationCheckpoint {
        //     digest: Felt::from(20u64),
        //     block_number: 20
        // }).is_some());
    }

    #[test]
    fn add_misaligned_test() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 7u64..16 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }
    #[test]
    #[should_panic]
    fn fail_on_non_contiguous_checkpoint_test() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in (7u64..16).step_by(2) {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }

    #[test]
    fn checkpoints_serialize_test() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in (5u64..17) {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        let checkpoints_before = checkpoints;
        checkpoints_before
            .to_file("../data/test_checkpoints.json")
            .unwrap();

        let from_file =
            AttestationCheckpoints::try_from_file("../data/test_checkpoints.json").unwrap();
        let checkpoints_after = from_file;
        //        checkpoints_after.tail().block_number = so9;
        assert_eq!(checkpoints_before.stabilized, checkpoints_after.stabilized);
        assert_eq!(checkpoints_before.tail(), checkpoints_after.tail());
        assert_eq!(
            checkpoints_before.stabilized_head(),
            checkpoints_after.stabilized_head()
        );
        assert_eq!(checkpoints_before.head(), checkpoints_after.head());
        println!("{}", checkpoints_after);
        println!("head: {:?}", checkpoints_after.head());
    }

    #[test]
    fn fail_on_empty_prepend_test() {
        let mut checkpoints = AttestationCheckpoints::new();

        let block_number = 4u64;
        let res = checkpoints.try_prepend(AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        });

        assert_eq!(res, Err(AttestationCheckpointError::PrependToUnstabilized));
    }
    #[test]
    fn prepend_to_unstabilized_test() {
        //        fn fail_on_prepend_to_unstabilized_test() {
        let mut checkpoints = AttestationCheckpoints::new();

        let block_number = 10u64;
        checkpoints
            .try_append(AttestationCheckpoint {
                digest: Felt::from(block_number),
                block_number: block_number.into(),
            })
            .unwrap();

        let block_number = 8u64;
        let res = checkpoints.try_prepend(AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        });

        //        assert_eq!(res, Err(AttestationCheckpointError::PrependToUnstabilized));
        assert_eq!(res, Ok(()));
    }

    #[test]
    fn prepend_test1() {
        let mut checkpoints = AttestationCheckpoints::new();

        let block_number = 8u64;
        checkpoints
            .try_append(AttestationCheckpoint {
                digest: Felt::from(block_number),
                block_number: block_number.into(),
            })
            .unwrap();

        let block_number = 4u64;
        checkpoints
            .try_prepend(AttestationCheckpoint {
                digest: Felt::from(block_number),
                block_number: block_number.into(),
            })
            .unwrap();

        println!("{}", checkpoints);
        //        assert_eq!(res, Err(AttestationCheckpointError::PrependToUnstabilized));
    }

    #[test]
    fn prepend_test2() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 5u64..17 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
            //            println!("{}", checkpoints);
        }

        let block_number = 4u64;
        let prepended = AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        };
        let res = checkpoints.try_prepend(prepended);
        assert_eq!(res, Ok(()));
        assert!(checkpoints.any(&prepended));

        let block_number = 0u64;
        let prepended = AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        };
        let res = checkpoints.try_prepend(prepended);
        assert_eq!(res, Ok(()));
        assert!(checkpoints.any(&prepended));

        println!("{}", checkpoints);
    }

    #[test]
    fn fail_on_prepend_after_tail() {
        //        fn fail_on_prepend_after_tail() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 5u64..17 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        println!("{}", checkpoints);

        let block_number = 8;
        let prepended = AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        };
        let res = checkpoints.try_prepend(prepended);
        println!("{}", checkpoints);
        assert_eq!(
            res,
            Err(AttestationCheckpointError::TailCheckpointExpected(
                4u64.into()
            ))
        );

        let block_number = 16;
        let prepended = AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        };
        let res = checkpoints.try_prepend(prepended);
        assert_eq!(
            res,
            Err(AttestationCheckpointError::TailCheckpointExpected(
                4u64.into()
            ))
        );
        println!("{}", checkpoints);
    }

    #[test]
    fn prepend_on_unstabilized() {
        //        fn fail_on_prepend_after_tail() {
        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 5u64..8 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        println!("{}", checkpoints);

        let block_number = 4u64;
        let prepended = AttestationCheckpoint {
            digest: Felt::from(block_number),
            block_number: block_number.into(),
        };
        let res = checkpoints.try_prepend(prepended);
        println!("{}", checkpoints);
        assert_eq!(res, Ok(()));
    }

    #[test]
    fn checkpoints_serialize_test2() {
        use std::fs::create_dir_all;
        create_dir_all("../data/execution-chain").unwrap();

        let mut checkpoints = AttestationCheckpoints::new();

        for block_number in 40u64..81 {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        for block_number in (4u64..37).rev().step_by(4) {
            checkpoints
                .try_prepend(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number: block_number.into(),
                })
                .unwrap();
        }
        let stabilized_before = checkpoints.stabilized.clone();
        checkpoints
            .to_file("../data/execution-chain/test_checkpoints.json")
            .unwrap();

        let from_file =
            AttestationCheckpoints::try_from_file("../data/execution-chain/test_checkpoints.json")
                .unwrap();
        let stabilized_after = from_file.stabilized.clone();
        assert_eq!(stabilized_before, stabilized_after);
        println!("{}", checkpoints);
    }
}
