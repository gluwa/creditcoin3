use anyhow::anyhow;
use starknet_crypto::{pedersen_hash, FieldElement};
use std::fmt::Debug;

#[derive(core::hash::Hash, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub struct StarknetFeltWrapped(pub FieldElement);

impl From<FieldElement> for StarknetFeltWrapped {
    fn from(felt: FieldElement) -> Self {
        Self(felt)
    }
}

impl From<u8> for StarknetFeltWrapped {
    fn from(n: u8) -> Self {
        Self(FieldElement::from(n))
    }
}

impl AsRef<FieldElement> for StarknetFeltWrapped {
    fn as_ref(&self) -> &FieldElement {
        &self.0
    }
}

impl From<StarknetFeltWrapped> for [u8; 32] {
    fn from(val: StarknetFeltWrapped) -> Self {
        val.0.to_bytes_be()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarknetPedersenHash;

impl mmr::traits::HashT for StarknetPedersenHash {
    type Output = StarknetFeltWrapped;

    fn hash(data: &[u8]) -> Self::Output {
        let felts = felts_from_bytes(data);

        array(&felts[..]).into()
    }

    fn concat_then_hash(felt_hashes: &[Self::Output]) -> Self::Output {
        array(felt_hashes).into()
    }
}

const U248_BYTE_COUNT: usize = 31;

#[must_use]
pub fn felts_from_bytes(bytes: &[u8]) -> Vec<FieldElement> {
    let chunks = bytes.chunks(U248_BYTE_COUNT);

    chunks
        .map(|chunk| {
            FieldElement::from_byte_slice_be(chunk)
                .expect("chunk length matches canonical length. qed")
        })
        .collect::<Vec<_>>()
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

pub fn array<T: AsRef<FieldElement>>(felts: &[T]) -> FieldElement {
    let maybe_zero_prefix = *felts[0].as_ref();
    let mut prev = maybe_zero_prefix;

    for felt in &felts[1..] {
        prev = pedersen_hash(&prev, felt.as_ref());
    }

    let len_felt = FieldElement::from_byte_slice_be(&u64_to_bytes_be((felts.len() - 1) as u64))
        .expect("length is less than canonical length. qed");

    //    println!("len: {}", len_felt.as_ref().to_string());
    pedersen_hash(prev.as_ref(), &len_felt)
}

fn u64_to_bytes_be(x: u64) -> [u8; 8] {
    let mut buf = [0u8; 8];

    buf[7] = (x & 0x0000_0000_0000_00ff) as u8;
    buf[6] = ((x & 0x0000_0000_0000_ff00) >> 8) as u8;
    buf[5] = ((x & 0x0000_0000_00ff_0000) >> 16) as u8;
    buf[4] = ((x & 0x0000_0000_ff00_0000) >> 24) as u8;
    buf[3] = ((x & 0x0000_00ff_0000_0000) >> 32) as u8;
    buf[2] = ((x & 0x0000_ff00_0000_0000) >> 40) as u8;
    buf[1] = ((x & 0x00ff_0000_0000_0000) >> 48) as u8;
    buf[0] = ((x & 0xff00_0000_0000_0000) >> 56) as u8;
    buf
}

#[cfg(test)]
mod tests {
    use super::{array, felt_from_dec_str, u64_to_bytes_be, FieldElement};
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

    // #[test]
    // fn array_3_elements_test() {
    //     let bytes_be = u64_to_bytes_be(0xa);
    //     let a = FieldElement::from_byte_slice_be(&bytes_be).unwrap();

    //     let bytes_be = u64_to_bytes_be(0xb);
    //     let b = FieldElement::from_byte_slice_be(&bytes_be).unwrap();

    //     let bytes_be = u64_to_bytes_be(0xc);
    //     let c = FieldElement::from_byte_slice_be(&bytes_be).unwrap();

    //     let h = array(&[a, b, c]);

    //     assert_eq!(
    //         h,
    //         // from Cairo0
    //         felt_from_dec_str(
    //             "-1387210446676157949284005763581452460269706036785075546825259478678905521525"
    //         )
    //         .unwrap()
    //     );
    // }
}
