use crate::block::Block;
use crate::dense_checkpoints::{DenseCheckpoints, DenseCheckpointsSerializable};
//use crate::{ATTESTATION_GENESIS, CHECKPOINT_INTERVAL};
use crate::AttestationChainParams;
use serde::{Deserialize, Serialize};
use utils::json_serializable::JsonSerializable;
use utils::Felt;

#[derive(Debug, PartialEq)]
pub enum AttestationCheckpointError {
    HeadCheckpointExpected(u64, u64),
    TailCheckpointExpected(u64),
    GenesisReached,
    DiscontinuedCheckpoints,
    PrependToUnstabilized,
    MisalignedStabilizedCheckpoint(u64),
    InvalidAttestationCheckpoint(u64),
    Other(String),
}

#[derive(PartialEq, Debug, Clone, Copy, Default)]
pub struct AttestationCheckpoint {
    block_number: u64,
    digest: Felt,
}

impl AttestationCheckpoint {
    pub fn try_from_block(
        params: AttestationChainParams,
        block_number: u64,
        digest: Felt,
    ) -> Option<Self> {
        params.index_for(block_number).map(|_| Self {
            block_number,
            digest,
        })
    }
    pub fn digest(&self) -> &Felt {
        &self.digest
    }
    pub fn n(&self) -> u64 {
        self.block_number
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
            Some(block_number_str) => block_number_str.parse().map_err(|_| ())?,
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

//pub type AttestationInterval = AttestationInterval<19504000, 4>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttestationInterval(pub(crate) u64, pub(crate) u64);

impl AttestationInterval {
    // pub fn interval_for(b: u64) -> Option<Self> {
    //     if b == GENESIS {
    //         return None;
    //     }
    //     AttestationCheckpoint::checkpoint_for(
    //         b - u64::from(Self::is_aligned(b)),
    //     )
    //     .and_then(|head| {
    //         head.checked_sub(INTERVAL as u64)
    //             .map(|tail| Self(tail, head))
    //     })
    // }
    // pub fn index(b: u64) -> Option<usize> {
    //     b.checked_sub(GENESIS)
    //         .map(|delta| (delta % INTERVAL as u64) as usize)
    // }

    // pub fn is_aligned(b: u64) -> bool {
    //     Self::index(b) == Some(0)
    // }

    pub fn tail(&self) -> u64 {
        self.0
    }
    pub fn head(&self) -> u64 {
        self.1
    }
    pub fn prev(&self, params: &AttestationChainParams) -> Option<Self> {
        params.interval_for(self.tail())
    }
    pub fn next(&self, params: &AttestationChainParams) -> Self {
        params
            .interval_for(self.head() + 1)
            .expect("can get next interval")
    }
}

#[derive(Debug, Clone, PartialEq)]
struct StabilizedCheckpoint(AttestationCheckpoint);

impl StabilizedCheckpoint {
    fn try_create(
        cp: AttestationCheckpoint,
        params: &AttestationChainParams,
    ) -> Result<Self, AttestationCheckpointError> {
        if params.is_aligned(cp.n()) {
            Ok(Self(cp))
        } else {
            Err(AttestationCheckpointError::MisalignedStabilizedCheckpoint(
                cp.n(),
            ))
        }
    }

    fn index(&self, params: &AttestationChainParams) -> usize {
        self.0
            .n()
            .checked_sub(params.genesis())
            .map(|delta| (delta / params.interval() as u64) as usize)
            .expect("stabilized checkpoint is aligned with respect to genesis")
    }

    fn n_from_index(index: usize, params: &AttestationChainParams) -> u64 {
        params.genesis() + (index * params.interval()) as u64
    }

    fn try_next_from(
        &self,
        cp: AttestationCheckpoint,
        params: &AttestationChainParams,
    ) -> Result<Self, AttestationCheckpointError> {
        let scp = Self::try_create(cp, params)?;

        if scp.index(params) == self.index(params) + 1 {
            Ok(scp)
        } else {
            Err(AttestationCheckpointError::HeadCheckpointExpected(
                Self::n_from_index(self.index(params) + 1, params),
                cp.n(),
            ))
        }
    }

    // fn try_prev_from(&self, cp: AttestationCheckpoint) -> Result<Self, AttestationCheckpointError> {
    //     let scp = Self::try_from(cp)?;
    //     if scp.index() + 1 == self.index() {
    //         Ok(scp)
    //     } else if self.index() == 0 {
    //         Err(AttestationCheckpointError::GenesisReached)
    //     } else {
    //         Err(AttestationCheckpointError::TailCheckpointExpected(
    //             Self::n_from_index(self.index() - 1),
    //         ))
    //     }
    // }
}

// impl TryFrom<AttestationCheckpoint>
//     for StabilizedCheckpoint
// {
//     type Error = AttestationCheckpointError;

//     fn try_from(
//         cp: AttestationCheckpoint,
//     ) -> Result<Self, AttestationCheckpointError> {
//         if AttestationInterval::is_aligned(cp.n()) {
//             Ok(Self(cp))
//         } else {
//             Err(AttestationCheckpointError::MisalignedStabilizedCheckpoint(
//                 cp.n(),
//             ))
//         }
//     }
// }

//#[derive(Debug, Default)]
#[derive(Debug)]
pub struct AttestationCheckpoints {
    params: AttestationChainParams,
    stabilized: Vec<Option<StabilizedCheckpoint>>,
    dense_checkpoints: DenseCheckpoints,
    tail: Option<StabilizedCheckpoint>,
}

impl AttestationCheckpoints {
    pub fn new(params: AttestationChainParams) -> Self {
        let interval = params.interval();

        Self {
            params,
            stabilized: Default::default(),
            dense_checkpoints: DenseCheckpoints::new(interval),
            tail: None,
        }
    }

    pub fn params(&self) -> AttestationChainParams {
        self.params
    }

    // pub fn interval(&self) -> usize {
    //     INTERVAL
    // }

    pub fn tail(&self) -> Option<u64> {
        self.tail.as_ref().map(|tail| tail.0.n())
    }
    pub fn head(&self) -> Option<u64> {
        self.dense_checkpoints.head()
    }
    pub fn stabilized_head(&self) -> Option<u64> {
        self.stabilized
            .last()
            .unwrap_or(&None)
            .as_ref()
            .map(|last| last.0.n())
    }
    pub fn checkpoint_for(&self, b: u64) -> Option<AttestationCheckpoint> {
        //        let bcp = AttestationCheckpoint::checkpoint_for(b)?;
        let bcp = self.params.checkpoint_number_for(b)?;

        let scp = AttestationCheckpoint::try_from_block(self.params, bcp, Default::default())
            .and_then(|cp| StabilizedCheckpoint::try_create(cp, &self.params).ok())?;
        //        println!("bcp: {bcp:?}, block: {b}, index: {}, stabilized len: {}", scp.index(), self.stabilized.len());
        let scp_index = scp.index(&self.params);
        if scp_index < self.stabilized.len() {
            self.stabilized[scp_index].as_ref().map(|scp| scp.0)
        } else {
            self.dense_checkpoints
                .checkpoint_for(b, &self.params)
                .copied()
        }
    }

    pub fn try_append(
        &mut self,
        cp: AttestationCheckpoint,
    ) -> Result<(), AttestationCheckpointError> {
        let stabilized_cp_opt = self.dense_checkpoints.try_append(cp, &self.params)?;
        stabilized_cp_opt
            .map(|stabilized_cp| self.try_append_stabilized(stabilized_cp))
            .unwrap_or_else(|| {
                if self.stabilized.is_empty() {
                    if let Some(head) = self.head() {
                        if let Some(future_stabilized) = self.params.checkpoint_number_for(head)
                        //                        AttestationCheckpoint::checkpoint_for(head)
                        {
                            let checkpoint = AttestationCheckpoint::try_from_block(
                                self.params,
                                future_stabilized,
                                Default::default(),
                            )
                            .expect("checkpoint can be created");

                            let future_stabilized =
                                StabilizedCheckpoint::try_create(checkpoint, &self.params)?;

                            self.stabilized = vec![None; future_stabilized.index(&self.params)];
                        }
                    }
                }
                Ok(())
            })
    }

    // pub fn try_prepend(
    //     &mut self,
    //     cp: AttestationCheckpoint,
    // ) -> Result<(), AttestationCheckpointError> {
    //     //        let tail = self.tail.as_ref().ok_or(AttestationCheckpointError::PrependToUnstabilized)?;
    //     let scp = match self.tail.as_ref() {
    //         Some(tail) => StabilizedCheckpoint::try_prev_from(tail, cp),
    //         // stabilized is empty, still can prepend if there are dense checkpoints
    //         None if self.head() > Some(cp.n()) => StabilizedCheckpoint::try_from(cp),
    //         None => Err(AttestationCheckpointError::PrependToUnstabilized),
    //     }?;
    //     self.stabilized[scp.index()] = Some(scp.clone());
    //     self.tail = Some(scp);
    //     Ok(())
    // }
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
                    .map(|stabilized_head| stabilized_head.try_next_from(cp, &self.params))
                    .unwrap_or_else(|| StabilizedCheckpoint::try_create(cp, &self.params))
            })
            // stabilized buffer is empty
            .unwrap_or_else(|| {
                let stabilized_checkpoint = StabilizedCheckpoint::try_create(cp, &self.params)?;
                // create stabilized vector back to genesis
                self.stabilized = vec![None; stabilized_checkpoint.index(&self.params)];

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
        let mut checkpoints = Self::new(checkpoints_json.params);

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
    params: AttestationChainParams,
    checkpoints: Vec<AttestationCheckpointSerializable>,
    dense_checkpoints: DenseCheckpointsSerializable,
    tail: AttestationCheckpointSerializable,
}

impl From<&AttestationCheckpoints> for AttestationCheckpointsSerializable {
    fn from(checkpoints: &AttestationCheckpoints) -> Self {
        Self {
            params: checkpoints.params,
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
    use crate::attestation_checkpoints::{AttestationCheckpoint, AttestationCheckpoints};
    use crate::ETH_ATTESTATION_CHAIN_PARAMS_DEV;
    use starknet_crypto::Felt;

    #[test]
    fn basic_append_test1() {
        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut checkpoints = AttestationCheckpoints::new(ETH_ATTESTATION_CHAIN_PARAMS_DEV);

        for block_number in genesis..8 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
                })
                .unwrap();
        }

        for block_number in 8 + genesis..21 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }

    #[test]
    fn basic_append_test2() {
        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut checkpoints = AttestationCheckpoints::new(ETH_ATTESTATION_CHAIN_PARAMS_DEV);

        for block_number in 8 + genesis..16 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
                })
                .unwrap();
        }

        for block_number in 16 + genesis..29 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }

    #[test]
    fn add_misaligned_test() {
        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut checkpoints = AttestationCheckpoints::new(ETH_ATTESTATION_CHAIN_PARAMS_DEV);

        for block_number in 7 + genesis..16 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }

    #[test]
    #[should_panic]
    fn fail_on_non_contiguous_checkpoint_test() {
        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut checkpoints = AttestationCheckpoints::new(ETH_ATTESTATION_CHAIN_PARAMS_DEV);

        for block_number in (7 + genesis..16 + genesis).step_by(2) {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
                })
                .unwrap();
        }
        println!("{}", checkpoints);
    }

    #[test]
    fn checkpoints_serialize_test() {
        use std::fs::create_dir_all;
        create_dir_all("../data").expect("Failed to create data directory");

        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut checkpoints = AttestationCheckpoints::new(ETH_ATTESTATION_CHAIN_PARAMS_DEV);

        for block_number in 5 + genesis..17 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
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
    fn checkpoints_serialize_test2() {
        use std::fs::create_dir_all;
        create_dir_all("../data/execution-chain").expect("Failed to create directory");

        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut checkpoints = AttestationCheckpoints::new(ETH_ATTESTATION_CHAIN_PARAMS_DEV);

        for block_number in 40 + genesis..81 + genesis {
            checkpoints
                .try_append(AttestationCheckpoint {
                    digest: Felt::from(block_number),
                    block_number,
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
