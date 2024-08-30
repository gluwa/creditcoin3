use crate::{
    attestation_checkpoints::{
        AttestationCheckpoint, AttestationCheckpointError, AttestationCheckpoints,
    },
    AttestationChainParams,
};

pub struct AttestationCheckpointsForDev {
    inner: AttestationCheckpoints,
    full_path: String,
}

impl Clone for AttestationCheckpointsForDev {
    fn clone(&self) -> Self {
        Self {
            inner: AttestationCheckpoints::new(self.inner.params()),
            full_path: self.full_path.clone(),
        }
    }
}

impl AttestationCheckpointsForDev {
    const FNAME: &'static str = "checkpoints.json";
    pub fn with_execution_chain_url(path: &str, params: AttestationChainParams) -> Self {
        use std::fs::create_dir_all;
        create_dir_all(path).unwrap();

        Self {
            inner: AttestationCheckpoints::new(params),
            full_path: path.to_owned() + "/" + Self::FNAME,
        }
    }
    pub fn full_path(&self) -> &str {
        &self.full_path
    }

    pub fn inner(&self) -> &AttestationCheckpoints {
        &self.inner
    }
    pub fn try_append(
        &mut self,
        cp: AttestationCheckpoint,
    ) -> Result<(), AttestationCheckpointError> {
        self.inner.try_append(cp)?;
        self.inner.to_file(&self.full_path)?;
        Ok(())
    }
    // pub fn try_prepend(&mut self, cp: AttestationCheckpoint,) -> Result<(), AttestationCheckpointError> {
    //     self.inner.try_prepend(cp)?;
    //     self.inner.to_file(&self.full_path)?;
    //     Ok(())
    // }
    pub fn poll(&mut self) -> Result<(), AttestationCheckpointError> {
        self.inner = AttestationCheckpoints::try_from_file(&self.full_path)?;
        Ok(())
    }
}
