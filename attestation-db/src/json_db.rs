use crate::{AttestationDB, AttestationDBImpl, AttestationDbError};
use attestation_chain::attestation_checkpoints::AttestationInterval;
use attestation_chain::attestation_fragment::AttestationFragment;
use attestation_chain::block::Block;
use attestation_chain::{ATTESTATION_GENESIS, CHECKPOINT_INTERVAL};
use ethereum_types::U256;
use serde::{Deserialize, Serialize};
use std::fs::create_dir_all;
use utils::json_serializable::JsonSerializable;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct AttestationJsonDBState {
    num_of_fragments: usize,
    head_checkpoint: U256,
}

impl JsonSerializable for AttestationJsonDBState {}

pub struct AttestationJsonDB {
    local_path: String,
    state_path: String,

    recent_fragment: AttestationFragment,
    state: AttestationJsonDBState,
}

impl AttestationJsonDB {
    pub fn try_create(local_path: &str) -> Result<Self, AttestationDbError> {
        let local_path = if local_path.ends_with('/') {
            From::from(local_path)
        } else {
            format!("{local_path}/")
        };

        create_dir_all(&local_path).map_err(|err| AttestationDbError::Other(format!("{err:?}")))?;

        let state_path = local_path.clone() + Self::STATE_STORAGE_LOCATION;
        let state = AttestationJsonDBState::try_from_file(&state_path).unwrap_or_default();
        let mut recent_fragment = AttestationFragment::default();
        if let Some(last_saved_fragment) =
            Self::get_saved_fragment_for(&local_path, state.head_checkpoint)
        {
            recent_fragment
                .try_append_block(
                    last_saved_fragment
                        .head()
                        .cloned()
                        .expect("full fragment has head"),
                )
                .expect("can append to empty fragment");
        }
        Ok(Self {
            local_path,
            state_path,
            //                genesis: From::from(genesis_block),
            recent_fragment,
            //                len: 0,
            state,
        })
    }
    pub fn local_path(&self) -> &str {
        &self.local_path
    }
}

impl AttestationDB for AttestationJsonDB {
    fn checkpoint_interval(&self) -> usize {
        CHECKPOINT_INTERVAL
    }

    fn genesis(&self) -> U256 {
        //        self.genesis
        ATTESTATION_GENESIS
    }

    fn len(&self) -> usize {
        self.state.num_of_fragments
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn reset(&mut self) -> Result<(), AttestationDbError> {
        std::fs::remove_dir_all(&self.local_path).map_err(|_| AttestationDbError::ResetFailure)?;

        *self.recent_fragment_mut() = Default::default();
        self.state = Default::default();

        // self.state.to_file(&self.state_path)
        //     .map_err(|err| AttestationDbError::Other(format!("unable to save state: {err:?}")))?;

        Ok(())
    }
    fn recent_fragment(&self) -> &AttestationFragment {
        &self.recent_fragment
    }

    fn get_fragment_for(&self, block_number: U256) -> Option<AttestationFragment> {
        Self::get_saved_fragment_for(&self.local_path, block_number).or_else(|| {
            (Self::key_for(block_number) == Self::fragment_key(self.recent_fragment())
                && self.recent_fragment().head()?.n() >= block_number)
                .then_some(self.recent_fragment().clone())
        })
    }

    fn fragment_for_exists(&self, block_number: U256) -> bool {
        Self::fragment_full_path_for(&self.local_path, block_number)
            .as_deref()
            .map(|path| std::fs::File::open(path).is_ok())
            .unwrap_or(false)
    }
    fn fragment_exists(&self, interval: &AttestationInterval) -> bool {
        Self::fragment_full_path_for(&self.local_path, interval.head())
            .map(|path| std::fs::File::open(path).is_ok())
            .unwrap_or(false)
    }
}

impl AttestationJsonDB {
    const STATE_STORAGE_LOCATION: &'static str = "state.json";
    const FRAGMENT_STORAGE_LOCATION: &'static str = "attestation_fragment.json";

    fn get_saved_fragment_for(local_path: &str, block_number: U256) -> Option<AttestationFragment> {
        Self::fragment_full_path_for(local_path, block_number)
            .as_deref()
            .map(AttestationFragment::try_from_file)
            .and_then(Result::<_, _>::ok)
    }

    // uses tail.block_number + self.checkpoint_interval() instead of just head.block_number
    // as head fragment may be not full
    // and the key must be constant no matter if the fragment is full or not
    fn fragment_key(fragment: &AttestationFragment) -> Option<U256> {
        fragment
            .tail()
            .and_then(|tail| Self::key_for(tail.n() + CHECKPOINT_INTERVAL as u64))
    }

    fn fragment_path_for(local_path: &str, block_number: U256) -> Option<String> {
        Self::key_for(block_number).map(|key| local_path.to_owned() + &format!("{}", key))
    }
    fn fragment_full_path_for(local_path: &str, block_number: U256) -> Option<String> {
        Self::fragment_path_for(local_path, block_number)
            .map(|path| path + "/" + Self::FRAGMENT_STORAGE_LOCATION)
    }

    fn save_fragment_and_state(
        &self,
        fragment: &AttestationFragment,
    ) -> Result<(), AttestationDbError> {
        let fragment_head = fragment.head().map(Block::n).expect("fragment is full");

        let path = Self::fragment_path_for(&self.local_path, fragment_head).ok_or(
            AttestationDbError::Other(format!(
                "unable to get key for block number {fragment_head}"
            )),
        )?;

        create_dir_all(path).map_err(|err| AttestationDbError::Other(format!("{err:?}")))?;

        let full_path = Self::fragment_full_path_for(&self.local_path, fragment_head).ok_or(
            AttestationDbError::Other(format!(
                "unable to get key for block number {fragment_head}"
            )),
        )?;

        fragment.to_file(&full_path).map_err(|err| {
            AttestationDbError::Other(format!("unable to save fragment: {err:?}"))
        })?;

        self.state
            .to_file(&self.state_path)
            .map_err(|err| AttestationDbError::Other(format!("unable to save state: {err:?}")))?;

        Ok(())
    }
}

impl AttestationDBImpl for AttestationJsonDB {
    fn commit(
        &mut self,
        fragment: AttestationFragment,
    ) -> Result<Box<AttestationFragment>, AttestationDbError> {
        let interval = fragment.interval().expect("full fragment defines interval");
        if self.fragment_exists(&interval) {
            return Err(AttestationDbError::FragmentAlreadySet(interval));
        }

        let prev_state = self.state.clone();

        let fragment_checkpoint = fragment
            .checkpoint()
            .expect("full fragment has checkpoint")
            .n();
        if self.state.head_checkpoint < fragment_checkpoint {
            self.state.head_checkpoint = fragment_checkpoint;
        }
        self.state.num_of_fragments += 1;

        match self.save_fragment_and_state(&fragment) {
            Ok(()) => Ok(Box::new(fragment)),
            Err(err) => {
                self.state = prev_state;
                Err(err)
            }
        }
    }
    fn recent_fragment_mut(&mut self) -> &mut AttestationFragment {
        &mut self.recent_fragment
    }
}

#[cfg(test)]
mod tests {
    use crate::json_db::AttestationJsonDB;
    use crate::AttestationDB;
    use crate::AttestationDbError;
    use crate::AttestationFragment;
    use crate::FullFragment;
    use attestation_chain::attestation_fragment::AttestationFragmentError;
    use attestation_chain::block::Block;
    use ethereum_types::U256;
    use std::sync::Mutex;

    const DB_PATH: &str = "../data/test_db";

    lazy_static::lazy_static! {
        static ref DB_LOCK: Mutex<()> = Mutex::new(());

        // static ref DB_INSTANCE: Mutex<Option<AttestationJsonDB>> = Mutex::new(
        //     Some(
        //         AttestationJsonDB::try_create(
        //             DB_PATH,
        //             42,
        //         ).unwrap()
        //     )
        // );

    }

    // fn recreate_db(db: &mut AttestationJsonDB) {
    //     if std::path::Path::new(DB_PATH).exists() {
    //         std::fs::remove_dir_all(DB_PATH).unwrap();
    //     }
    //     *db =
    //         AttestationJsonDB::try_create(
    //             DB_PATH,
    //             42,
    //         ).unwrap();
    // }

    fn recreate_instance() -> AttestationJsonDB {
        if std::path::Path::new(DB_PATH).exists() {
            std::fs::remove_dir_all(DB_PATH).unwrap();
        }

        AttestationJsonDB::try_create(DB_PATH).unwrap()
    }
    fn load_instance() -> AttestationJsonDB {
        AttestationJsonDB::try_create(DB_PATH).unwrap()
    }

    #[test]
    fn key_for_block_test() {
        let _lock = DB_LOCK.lock().unwrap();

        let block = U256::from(42);
        let key = AttestationJsonDB::key_for(block);
        assert_eq!(key, None);
        println!("key({block}) = {:?}", key);

        let block = U256::from(45);
        let key = AttestationJsonDB::key_for(block).unwrap();
        assert_eq!(key, 0.into());
        println!("key({block}) = {:?}", key);

        let block = 46u64;
        let key = AttestationJsonDB::key_for(block.into()).unwrap();
        assert_eq!(key, 0.into());
        println!("key({block}) = {:?}", key);

        let block = 50u64;
        let key = AttestationJsonDB::key_for(block.into()).unwrap();
        assert_eq!(key, 1.into());
        println!("key({block}) = {:?}", key);

        let block = 54u64;
        let key = AttestationJsonDB::key_for(block.into()).unwrap();
        assert_eq!(key, 2.into());
        println!("key({block}) = {:?}", key);
    }

    #[test]
    fn fail_set_misaligned_fragment_test() {
        let _lock = DB_LOCK.lock().unwrap();

        let mut fragment = AttestationFragment::default();

        let res = fragment.try_append_block(Block::new(43.into(), 0u64.into(), 0u64.into()));

        assert!(matches!(
            res,
            Err(AttestationFragmentError::MisalignedBlock(_))
        ));
    }

    #[test]
    fn fail_set_repeated_fragment_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        let mut fragment = AttestationFragment::default();
        fragment
            .try_append_block(Block::new(50.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(51.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(52.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(53.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(54.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let full_fragment = FullFragment::try_from(&fragment).unwrap();

        let res = db.set_fragment(full_fragment);
        //        println!("!!! {res:?}");
        assert!(res.is_ok());

        let mut fragment = AttestationFragment::default();
        fragment
            .try_append_block(Block::new(50.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(51.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(52.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(53.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(54.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let full_fragment = FullFragment::try_from(&fragment).unwrap();

        assert!(matches!(
            db.set_fragment(full_fragment),
            Err(AttestationDbError::FragmentAlreadySet(_))
        ));
    }

    #[test]
    fn set_two_fragments_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        let mut fragment = AttestationFragment::default();
        fragment
            .try_append_block(Block::new(50.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(51.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(52.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(53.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(54.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let full_fragment = FullFragment::try_from(&fragment).unwrap();
        let res = db.set_fragment(full_fragment);
        //        println!("!!! {res:?}");
        assert!(res.is_ok());
        assert_eq!(db.len(), 1);

        let mut fragment = AttestationFragment::default();
        fragment
            .try_append_block(Block::new(54.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(55.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(56.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(57.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(58.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let full_fragment = FullFragment::try_from(&fragment).unwrap();
        db.set_fragment(full_fragment).unwrap();
        assert_eq!(db.len(), 2);

        println!("KEY = {:?}", AttestationJsonDB::key_for(58.into()));
        let fr = AttestationJsonDB::get_fragment_for(&db, 58.into()).unwrap();
        //        println!("!!!! ({}, {})", fr.tail().unwrap().block_number, fr.head().unwrap().block_number);
        assert_eq!(fr.tail().map(Block::n), Some(54.into()));
        assert_eq!(fr.head().map(Block::n), Some(58.into()));
        //        db.flush();
    }

    #[test]
    fn append_block_as_genenis_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        let res = db.try_append_block(Block::new(42.into(), 0u64.into(), 0u64.into()));
        assert!(res.is_ok());
    }
    #[test]
    fn fail_appending_block_as_genenis_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        let res = db.try_append_block(Block::new(43.into(), 0u64.into(), 0u64.into()));
        assert!(matches!(
            res,
            Err(AttestationDbError::MisalignedBlockDiscarded(_))
        ));
    }
    #[test]
    fn append_blocks_from_genesis_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        db.try_append_block(Block::new(42.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        assert_eq!(db.len(), 0);
        db.try_append_block(Block::new(43.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        db.try_append_block(Block::new(44.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        db.try_append_block(Block::new(45.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        assert_eq!(db.len(), 0);

        db.try_append_block(Block::new(46.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        assert_eq!(db.len(), 1);
        db.try_append_block(Block::new(47.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        db.try_append_block(Block::new(48.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        db.try_append_block(Block::new(49.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        assert_eq!(db.len(), 1);

        db.try_append_block(Block::new(50.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        assert_eq!(db.len(), 2);
    }

    #[test]
    fn set_fragments_then_append_blocks_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        let mut fragment = AttestationFragment::default();
        fragment
            .try_append_block(Block::new(50.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(51.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(52.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(53.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(54.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let res = db.set_fragment(FullFragment::try_from(&fragment).unwrap());
        //        println!("!!! {res:?}");
        assert!(res.is_ok());
        assert_eq!(db.len(), 1);

        let mut fragment = AttestationFragment::default();
        fragment
            .try_append_block(Block::new(42.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(43.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(44.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(45.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        fragment
            .try_append_block(Block::new(46.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        db.set_fragment(FullFragment::try_from(&fragment).unwrap())
            .unwrap();
        assert_eq!(db.len(), 2);

        let fr = db.get_fragment_for(53.into()).unwrap();
        assert_eq!(fr.head().map(Block::n), Some(54.into()));

        db.try_append_block(Block::new(55.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let prev_len = db.len();
        let _prev_head_fragment = db.recent_fragment().clone();
        drop(db);

        let db = load_instance();
        assert_eq!(prev_len, db.len());
    }

    #[test]
    fn load_db_state_test() {
        let _lock = DB_LOCK.lock().unwrap();
        let mut db = recreate_instance();

        db.try_append_block(Block::new(50.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        db.try_append_block(Block::new(51.into(), 0u64.into(), 0u64.into()))
            .unwrap();
        db.try_append_block(Block::new(52.into(), 0u64.into(), 0u64.into()))
            .unwrap();

        let prev_len = db.len();

        drop(db);

        let db = load_instance();
        assert_eq!(prev_len, db.len());
    }
}
