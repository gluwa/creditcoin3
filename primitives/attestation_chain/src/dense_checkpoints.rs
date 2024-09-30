use crate::attestation_checkpoints::{
    AttestationCheckpoint, AttestationCheckpointError, AttestationCheckpointSerializable,
};
use crate::AttestationChainParams;
use serde::{Deserialize, Serialize};

fn cp_index(params: &AttestationChainParams, cp: &AttestationCheckpoint) -> Option<usize> {
    params.index_in_interval_for(cp.n())
}

type DenseCheckpointBuffer = Vec<Option<AttestationCheckpoint>>;

#[derive(Debug, Clone)]
pub(crate) struct DenseCheckpoints {
    head: Option<u64>,
    buffers: [DenseCheckpointBuffer; 2],
    curr_buf: usize,
}

impl DenseCheckpoints {
    pub fn new(interval: usize) -> Self {
        Self {
            head: Default::default(),
            buffers: [vec![None; interval], vec![None; interval]],
            curr_buf: Default::default(),
        }
    }

    pub fn try_append(
        &mut self,
        cp: AttestationCheckpoint,
        params: &AttestationChainParams,
    ) -> Result<Option<AttestationCheckpoint>, AttestationCheckpointError> {
        let cp_index = cp_index(params, &cp).ok_or(
            AttestationCheckpointError::InvalidAttestationCheckpoint(cp.n()),
        )?;

        if let Some(head) = self.head {
            if head + 1 != cp.n() {
                return Err(AttestationCheckpointError::HeadCheckpointExpected(
                    head + 1,
                    cp.n(),
                ));
            }
        }
        let output = if cp_index == 0 {
            if self.is_full() {
                *self.prev_mut() = vec![None; params.interval()]
            }
            self.swap();
            Some(cp)
        } else {
            self.curr_mut()[cp_index] = Some(cp);
            None
        };

        self.head = Some(cp.n());
        Ok(output)
    }

    #[cfg(test)]
    pub fn any(&self, cp: &AttestationCheckpoint, params: &AttestationChainParams) -> bool {
        cp_index(params, cp)
            .map(|index| {
                self.prev()[index].as_ref() == Some(cp) || self.curr()[index].as_ref() == Some(cp)
            })
            .unwrap_or(false)
    }

    pub fn checkpoint_for(
        &self,
        b: u64,
        params: &AttestationChainParams,
    ) -> Option<&AttestationCheckpoint> {
        let index = params.index_in_interval_for(b)?;

        if self.prev()[index].map(|cp| cp.n()) == Some(b) {
            self.prev()[index].as_ref()
        } else if self.curr()[index].map(|cp| cp.n()) == Some(b) {
            self.curr()[index].as_ref()
        } else {
            None
        }
    }

    pub fn head(&self) -> Option<u64> {
        self.head
    }
}

impl DenseCheckpoints {
    fn curr(&self) -> &DenseCheckpointBuffer {
        &self.buffers[self.curr_buf]
    }
    fn curr_mut(&mut self) -> &mut DenseCheckpointBuffer {
        &mut self.buffers[self.curr_buf]
    }
    fn prev(&self) -> &DenseCheckpointBuffer {
        &self.buffers[self.curr_buf ^ 0x1]
    }
    fn prev_mut(&mut self) -> &mut DenseCheckpointBuffer {
        &mut self.buffers[self.curr_buf ^ 0x1]
    }
    fn swap(&mut self) {
        self.curr_buf ^= 0x1;
    }
    fn is_full(&self) -> bool {
        self.curr().last().is_some()
    }
}

type DenseCheckpointsSerializableBuffer = Vec<AttestationCheckpointSerializable>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DenseCheckpointsSerializable {
    head: Option<u64>,
    buffers: [DenseCheckpointsSerializableBuffer; 2],
    curr_buf: usize,
}

impl From<&DenseCheckpoints> for DenseCheckpointsSerializable {
    fn from(dense_checkpoints: &DenseCheckpoints) -> Self {
        let buf0 = dense_checkpoints.buffers[0]
            .iter()
            .map(Option::<_>::as_ref)
            .map(From::from)
            .collect::<Vec<_>>();
        let buf1 = dense_checkpoints.buffers[1]
            .iter()
            .map(Option::<_>::as_ref)
            .map(From::from)
            .collect::<Vec<_>>();
        Self {
            head: dense_checkpoints.head,
            buffers: [buf0, buf1],
            curr_buf: dense_checkpoints.curr_buf,
        }
    }
}

impl TryFrom<DenseCheckpointsSerializable> for DenseCheckpoints {
    type Error = ();

    fn try_from(checkpoints_json: DenseCheckpointsSerializable) -> Result<Self, Self::Error> {
        let buf0 = checkpoints_json.buffers[0]
            .iter()
            .map(TryFrom::try_from)
            .collect::<Result<Vec<_>, Self::Error>>()?;
        let buf1 = checkpoints_json.buffers[1]
            .iter()
            .map(TryFrom::try_from)
            .collect::<Result<Vec<_>, Self::Error>>()?;

        Ok(Self {
            head: checkpoints_json.head,
            buffers: [buf0, buf1],
            curr_buf: checkpoints_json.curr_buf,
        })
    }
}

impl std::fmt::Display for DenseCheckpoints {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "[")?;
        for cp in self.prev() {
            write!(
                f,
                "{},",
                cp.map(|cp| cp.n().to_string()).unwrap_or("_".to_string())
            )?;
        }
        write!(f, "][")?;
        for cp in self.curr() {
            write!(
                f,
                "{},",
                cp.map(|cp| cp.n().to_string()).unwrap_or("_".to_string())
            )?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::attestation_checkpoints::AttestationCheckpoint;
    use crate::dense_checkpoints::DenseCheckpoints;
    use crate::ETH_ATTESTATION_CHAIN_PARAMS_DEV;

    #[test]
    fn basic_dense_append_test4() {
        let interval = ETH_ATTESTATION_CHAIN_PARAMS_DEV.interval();
        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut dcps = DenseCheckpoints::new(interval);

        for i in 2 + genesis..8 + genesis {
            let cp = AttestationCheckpoint::try_from_block(
                ETH_ATTESTATION_CHAIN_PARAMS_DEV,
                i,
                0u64.into(),
            )
            .unwrap();
            let res = dcps
                .try_append(cp, &ETH_ATTESTATION_CHAIN_PARAMS_DEV)
                .unwrap();
            if i != genesis + interval as u64 {
                assert!(res.is_none());
                assert!(dcps.any(&cp, &ETH_ATTESTATION_CHAIN_PARAMS_DEV));
            } else {
                assert_eq!(res.map(|cp| cp.n()), Some(genesis + interval as u64));
            }
        }
        println!("{}", dcps);
    }
    #[test]
    fn basic_dense_append_test6() {
        let interval = ETH_ATTESTATION_CHAIN_PARAMS_DEV.interval();
        let genesis = ETH_ATTESTATION_CHAIN_PARAMS_DEV.genesis();

        let mut dcps = DenseCheckpoints::new(interval);

        for i in 2 + genesis..8 + genesis {
            AttestationCheckpoint::try_from_block(ETH_ATTESTATION_CHAIN_PARAMS_DEV, i, 0u64.into())
                .unwrap();
        }
        assert_eq!(
            dcps.try_append(
                AttestationCheckpoint::try_from_block(
                    ETH_ATTESTATION_CHAIN_PARAMS_DEV,
                    genesis + 8,
                    0u64.into()
                )
                .unwrap(),
                &ETH_ATTESTATION_CHAIN_PARAMS_DEV
            )
            .unwrap()
            .map(|cp| cp.n()),
            Some(genesis + 8)
        );
        for i in 9 + genesis..12 + genesis {
            let cp = AttestationCheckpoint::try_from_block(
                ETH_ATTESTATION_CHAIN_PARAMS_DEV,
                i,
                0u64.into(),
            )
            .unwrap();
            assert!(dcps
                .try_append(cp, &ETH_ATTESTATION_CHAIN_PARAMS_DEV)
                .unwrap()
                .is_none());
            assert!(dcps.any(&cp, &ETH_ATTESTATION_CHAIN_PARAMS_DEV));
        }
        let cp = AttestationCheckpoint::try_from_block(
            ETH_ATTESTATION_CHAIN_PARAMS_DEV,
            genesis + 12,
            0u64.into(),
        )
        .unwrap();
        assert_eq!(
            dcps.try_append(cp, &ETH_ATTESTATION_CHAIN_PARAMS_DEV)
                .unwrap()
                .map(|cp| cp.n()),
            Some(genesis + 12)
        );
        assert!(!dcps.any(&cp, &ETH_ATTESTATION_CHAIN_PARAMS_DEV));
        println!("{}", dcps);
    }

    // #[test]
    // fn basic_dense_append_test7() {
    //     let mut dcps = DenseCheckpoints::default();

    //     for i in 4..8 {
    //         let cp = AttestationCheckpoint::try_from_block(i, 0u64.into()).unwrap();
    //         assert!(dcps.try_append(cp).unwrap().is_none());
    //     }
    //     // assert_eq!(
    //     //     dcps.try_append(AttestationCheckpoint::new(8, 0u64.into())).unwrap().map(|cp| cp.n()),
    //     //     Some(4)
    //     // );
    //     println!("{}", dcps);
    // }
}
