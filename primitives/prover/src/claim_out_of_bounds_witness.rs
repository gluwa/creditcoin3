use sp_std::{vec, vec::Vec};
use utils::block_item_traits::{BlockItem, BlockItemIdentifier};

#[derive(PartialEq, Clone, Default, Debug)]
pub struct ClaimOutOfBoundsWitness(BlockItemIdentifier);

impl BlockItem for ClaimOutOfBoundsWitness {
    fn id(&self) -> &BlockItemIdentifier {
        &self.0
    }

    fn tx_type(&self) -> Option<u8> {
        unreachable!("must not be used");
    }

    fn payload_bytes(&self) -> Vec<u8> {
        // let merkle_leaf_count = self.id().index();
        // rlp::encode(&merkle_leaf_count).to_vec()
        rlp::encode(&vec![]).to_vec()

        //        self.1.to_be_bytes().to_vec()
    }
}

// pub struct ClaimOutOfBoundsWitness(BlockItemIdentifier, u64);

// impl ClaimOutOfBoundsWitness {
//     pub fn new(id: BlockItemIdentifier, merkle_tree_height: u64) -> Self {
//         Self(id, merkle_tree_height)
//     }
// }

// impl BlockItem for ClaimOutOfBoundsWitness {
//     fn id(&self) -> &BlockItemIdentifier {
//         &self.0
//     }

//     fn tx_type(&self) -> Option<u8> {
//         unreachable!("must not be used");
//     }

//     fn payload_bytes(&self) -> Vec<u8> {
//         // let merkle_leaf_count = self.id().index();
//         // rlp::encode(&merkle_leaf_count).to_vec()
//         rlp::encode(&vec![]).to_vec()

// //        self.1.to_be_bytes().to_vec()
//     }
// }
