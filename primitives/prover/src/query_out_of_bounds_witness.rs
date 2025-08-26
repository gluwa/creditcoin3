use sp_std::{vec, vec::Vec};
use utils::block_item_traits::{BlockItem, BlockItemIdentifier};

#[derive(PartialEq, Clone, Default, Debug)]
pub struct QueryOutOfBoundsWitness(BlockItemIdentifier);

impl BlockItem for QueryOutOfBoundsWitness {
    fn id(&self) -> &BlockItemIdentifier {
        &self.0
    }

    fn payload_bytes(&self) -> Vec<u8> {
        // let merkle_leaf_count = self.id().index();
        // rlp::encode(&merkle_leaf_count).to_vec()
        rlp::encode(&vec![]).to_vec()

        //        self.1.to_be_bytes().to_vec()
    }

    fn tx_type(&self) -> Option<u8> {
        unreachable!("must not be used");
    }
}
