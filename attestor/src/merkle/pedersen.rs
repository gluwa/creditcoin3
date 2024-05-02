use anyhow::anyhow;
use starknet_crypto::{pedersen_hash, FieldElement};
use std::hash::{BuildHasher, Hash};

use super::tree::TElement;

#[derive(Clone, Debug, PartialEq, Eq, Default, PartialOrd, Ord, Hash)]
pub struct StarknetPedersenHash(pub FieldElement);

impl std::hash::Hasher for StarknetPedersenHash {
    fn finish(&self) -> u64 {
        // FIXME: contract is broken by design
        unimplemented!(
            "Hasher's contract (finish function is not used) is deliberately broken by design"
        )
    }

    fn write(&mut self, bytes: &[u8]) {
        let felts = felts_from_bytes(bytes);
        let felt = hash(&felts);

        self.0 = felt;
    }
}

pub struct StarknetPedersenHasher;

impl BuildHasher for StarknetPedersenHasher {
    type Hasher = StarknetPedersenHash;

    fn build_hasher(&self) -> Self::Hasher {
        StarknetPedersenHash(FieldElement::default())
    }
}

impl merkletree::hash::Algorithm<TElement> for StarknetPedersenHash {
    fn hash(&mut self) -> TElement {
        TElement(Vec::from(self.0.to_bytes_be()))
    }
}

//const U64_BYTE_COUNT: usize = 8;
const CHUNK_SIZE: usize = 31;

#[must_use]
pub fn felts_from_bytes(bytes: &[u8]) -> Vec<FieldElement> {
    let num_chunks = (bytes.len() + CHUNK_SIZE - 1) / CHUNK_SIZE; // Calculate the number of chunks needed
    let mut felts = Vec::with_capacity(num_chunks); // Pre-allocate memory for the felts vector

    for chunk in bytes.chunks(CHUNK_SIZE) {
        let field_element = FieldElement::from_byte_slice_be(chunk)
            .expect("chunk length matches canonical length. qed");
        felts.push(field_element);
    }

    felts
}

pub fn hash<T: AsRef<FieldElement>>(felts: &[T]) -> FieldElement {
    let mut prev = *felts[0].as_ref(); // Clone the first element as the initial accumulator

    for felt in &felts[1..] {
        prev = pedersen_hash(&prev, felt.as_ref());
    }

    let len_felt = FieldElement::from_byte_slice_be(&u64_to_bytes_be((felts.len() - 1) as u64))
        .expect("length is less than canonical length. qed");

    pedersen_hash(&prev, &len_felt)
}

fn u64_to_bytes_be(x: u64) -> [u8; 8] {
    x.to_be_bytes()
}

#[allow(dead_code)]
pub fn felt_from_dec_str(s: &str) -> anyhow::Result<FieldElement> {
    match FieldElement::from_dec_str(s) {
        Ok(x) => Ok(x),
        Err(_) if s.starts_with('-') => {
            let neg_x = FieldElement::from_dec_str(&s[1..]).map_err(|err| anyhow!("{}", err))?;
            Ok(FieldElement::ZERO - neg_x)
        }
        Err(err) => Err(anyhow!("{}", err)),
    }
}

#[cfg(test)]
mod tests {
    use super::{hash, u64_to_bytes_be, FieldElement};
    use starknet_crypto::pedersen_hash;

    #[test]
    fn pedersen2_test() {
        let bytes_be = u64_to_bytes_be(0x0000_0000_0000_0001);
        println!("bytes_be: {bytes_be:X?}");
        let a = FieldElement::from_byte_slice_be(&bytes_be).unwrap();
        println!("a: {a:X?}");

        let bytes_be = u64_to_bytes_be(0x0000_0000_0000_0002);
        println!("bytes_be: {bytes_be:X?}");
        let b = FieldElement::from_byte_slice_be(&bytes_be).unwrap();
        println!("b: {b:X?}");

        let h = pedersen_hash(&a, &b);
        println!("hash: {h:X?}");
        assert_eq!(
            h.to_bytes_be(),
            // taken from Golang's pedersen(a, b)
            &hex::decode("05bb9440e27889a364bcb678b1f679ecd1347acdedcbf36e83494f857cc58026")
                .unwrap()[..]
        );
    }

    #[test]
    fn pedersen2_test1() {
        let bytes_be = u64_to_bytes_be(0x0807_0605_0403_0201);
        println!("bytes_be: {bytes_be:X?}");
        let a = FieldElement::from_byte_slice_be(&bytes_be).unwrap();
        println!("a: {a:X?}");

        let bytes_be = u64_to_bytes_be(0x8070_6050_4030_2010);
        println!("bytes_be: {bytes_be:X?}");
        let b = FieldElement::from_byte_slice_be(&bytes_be).unwrap();
        println!("b: {b:X?}");

        let h = pedersen_hash(&a, &b);
        println!("hash: {h:X?}");
        assert_eq!(
            h.to_bytes_be(),
            // taken from Golang's pedersen(a, b)
            &hex::decode("05bbe990671c3e539518346a7513a60df1697e850540feb72f4377c061801be1")
                .unwrap()[..]
        );
    }

    #[test]
    fn pedersen_array_3_elements_test() {
        let bytes_be = u64_to_bytes_be(0xa);
        let a = FieldElement::from_byte_slice_be(&bytes_be).unwrap();

        let bytes_be = u64_to_bytes_be(0xb);
        let b = FieldElement::from_byte_slice_be(&bytes_be).unwrap();

        let bytes_be = u64_to_bytes_be(0xc);
        let c = FieldElement::from_byte_slice_be(&bytes_be).unwrap();

        let h = hash(&[a, b, c]);

        assert_eq!(
            h.to_bytes_be(),
            &hex::decode("00a4290678df78465a22eda0abe05b66b43096cc49bda2d138ae9b46f468b1be")
                .unwrap()[..]
        );
    }
}
