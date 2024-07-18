use serde::{Deserialize, Serialize};
//use common::json_serializable::JsonSerializable;
use crate::attestation_checkpoints::AttestationCheckpointError;
use crate::attestation_checkpoints::AttestationCheckpointSerializable;
use crate::attestation_checkpoints::{AttestationCheckpoint, AttestationInterval};
use crate::CHECKPOINT_INTERVAL;
use ethereum_types::U256;

fn cp_index(cp: &AttestationCheckpoint) -> Option<usize> {
    AttestationInterval::index(cp.n())
}

type DenseCheckpointBuffer = [Option<AttestationCheckpoint>; CHECKPOINT_INTERVAL];

#[derive(Default, Debug, Clone)]
pub(crate) struct DenseCheckpoints {
    head: Option<U256>,
    buffers: [DenseCheckpointBuffer; 2],
    curr_buf: usize,
}

impl DenseCheckpoints {
    pub fn try_append(
        &mut self,
        cp: AttestationCheckpoint,
    ) -> Result<Option<AttestationCheckpoint>, AttestationCheckpointError> {
        let cp_index = cp_index(&cp).ok_or(
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
                *self.prev_mut() = DenseCheckpointBuffer::default();
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

    pub fn any(&self, cp: &AttestationCheckpoint) -> bool {
        cp_index(cp)
            .map(|index| {
                self.prev()[index].as_ref() == Some(cp) || self.curr()[index].as_ref() == Some(cp)
            })
            .unwrap_or(false)
    }

    pub fn checkpoint_for(&self, b: U256) -> Option<&AttestationCheckpoint> {
        let index = AttestationInterval::index(b)?;

        if self.prev()[index].map(|cp| cp.n()) == Some(b) {
            self.prev()[index].as_ref()
        } else if self.curr()[index].map(|cp| cp.n()) == Some(b) {
            self.curr()[index].as_ref()
        } else {
            None
        }
    }

    pub fn head(&self) -> Option<U256> {
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

type DenseCheckpointsSerializableBuffer = [AttestationCheckpointSerializable; CHECKPOINT_INTERVAL];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DenseCheckpointsSerializable {
    head: Option<U256>,
    buffers: [DenseCheckpointsSerializableBuffer; 2],
    curr_buf: usize,
}

impl From<&DenseCheckpoints> for DenseCheckpointsSerializable {
    fn from(dense_checkpoints: &DenseCheckpoints) -> Self {
        let buf0 = dense_checkpoints.buffers[0]
            .iter()
            .map(Option::<_>::as_ref)
            .map(From::from)
            .collect::<Vec<_>>()
            .try_into()
            .expect("both arrays have same sizes");
        let buf1 = dense_checkpoints.buffers[1]
            .iter()
            .map(Option::<_>::as_ref)
            .map(From::from)
            .collect::<Vec<_>>()
            .try_into()
            .expect("both arrays have same sizes");
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
            .collect::<Result<Vec<_>, Self::Error>>()?
            .try_into()
            .expect("both arrays have same sizes");
        let buf1 = checkpoints_json.buffers[1]
            .iter()
            .map(TryFrom::try_from)
            .collect::<Result<Vec<_>, Self::Error>>()?
            .try_into()
            .expect("both arrays have same sizes");

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

    #[ignore]
    #[test]
    fn basic_dense_append_test4() {
        let mut dcps = DenseCheckpoints::default();

        for i in 2..8 {
            let cp = AttestationCheckpoint::try_from_block(i.into(), 0u64.into()).unwrap();
            let res = dcps.try_append(cp).unwrap();
            if i != 4 {
                assert!(res.is_none());
                assert!(dcps.any(&cp));
            } else {
                assert_eq!(res.map(|cp| cp.n()), Some(4.into()));
            }
        }
        println!("{}", dcps);
    }
    #[ignore]
    #[test]
    fn basic_dense_append_test6() {
        let mut dcps = DenseCheckpoints::default();

        for i in 2..8 {
            AttestationCheckpoint::try_from_block(i.into(), 0u64.into()).unwrap();
        }
        assert_eq!(
            dcps.try_append(AttestationCheckpoint::try_from_block(8.into(), 0u64.into()).unwrap())
                .unwrap()
                .map(|cp| cp.n()),
            Some(8.into())
        );
        for i in 9..12 {
            let cp = AttestationCheckpoint::try_from_block(i.into(), 0u64.into()).unwrap();
            assert!(dcps.try_append(cp).unwrap().is_none());
            assert!(dcps.any(&cp));
        }
        let cp = AttestationCheckpoint::try_from_block(12.into(), 0u64.into()).unwrap();
        assert_eq!(
            dcps.try_append(cp).unwrap().map(|cp| cp.n()),
            Some(12.into())
        );
        assert!(!dcps.any(&cp));
        println!("{}", dcps);
    }

    // #[test]
    // fn basic_dense_append_test7() {
    //     let mut dcps = DenseCheckpoints::default();

    //     for i in 4..8 {
    //         let cp = AttestationCheckpoint::try_from_block(i, 0u64.into()).unwrap();
    //         assert!(dcps.try_append(cp.clone()).unwrap().is_none());
    //     }
    //     // assert_eq!(
    //     //     dcps.try_append(AttestationCheckpoint::new(8, 0u64.into())).unwrap().map(|cp| cp.n()),
    //     //     Some(4)
    //     // );
    //     println!("{}", dcps);
    // }
}
