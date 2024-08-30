use crate::utils::felts_from_bytes;
use core::fmt::Debug;
use starknet_crypto::{pedersen_hash, Felt};

#[derive(core::hash::Hash, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub struct StarknetFeltWrapped(pub Felt);

impl From<Felt> for StarknetFeltWrapped {
    fn from(felt: Felt) -> Self {
        Self(felt)
    }
}

impl From<u8> for StarknetFeltWrapped {
    fn from(n: u8) -> Self {
        Self(Felt::from(n))
    }
}

impl AsRef<Felt> for StarknetFeltWrapped {
    fn as_ref(&self) -> &Felt {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarknetPedersenHash;

impl mmr::traits::HashT for StarknetPedersenHash {
    type Output = StarknetFeltWrapped;

    fn hash(data: &[u8]) -> Self::Output {
        let felts = felts_from_bytes(data);

        pedersen_array(&felts[..]).into()
    }

    fn concat_then_hash(felt_hashes: &[Self::Output]) -> Self::Output {
        pedersen_array(felt_hashes).into()
    }
}

pub fn pedersen_array<T: AsRef<Felt>>(felts: &[T]) -> Felt {
    let maybe_zero_prefix = *felts[0].as_ref();
    let mut prev = maybe_zero_prefix;

    for felt in &felts[1..] {
        prev = pedersen_hash(&prev, felt.as_ref());
    }

    let len_felt = Felt::from_bytes_be_slice(&u64_to_bytes_be((felts.len()) as u64));

    pedersen_hash(prev.as_ref(), &len_felt)
}

fn u64_to_bytes_be(x: u64) -> [u8; 8] {
    let mut buf = [0u8; 8];

    buf[7] = (x & 0x00000000000000ff) as u8;
    buf[6] = ((x & 0x000000000000ff00) >> 8) as u8;
    buf[5] = ((x & 0x0000000000ff0000) >> 16) as u8;
    buf[4] = ((x & 0x00000000ff000000) >> 24) as u8;
    buf[3] = ((x & 0x000000ff00000000) >> 32) as u8;
    buf[2] = ((x & 0x0000ff0000000000) >> 40) as u8;
    buf[1] = ((x & 0x00ff000000000000) >> 48) as u8;
    buf[0] = ((x & 0xff00000000000000) >> 56) as u8;
    buf
}

#[cfg(test)]
mod tests {
    use crate::pedersen_hash::{pedersen_array, u64_to_bytes_be, Felt};
    use crate::utils::felt_from_dec_str;

    use starknet_crypto::pedersen_hash;

    #[test]
    fn pedersen2_test() {
        let bytes_be = u64_to_bytes_be(0x0000000000000001);
        let a = Felt::from_bytes_be_slice(&bytes_be);

        let bytes_be = u64_to_bytes_be(0x0000000000000002);
        let b = Felt::from_bytes_be_slice(&bytes_be);

        let h = pedersen_hash(&a, &b);
        assert_eq!(
            h.to_bytes_be(),
            // taken from Golang's pedersen(a, b)
            &hex::decode("05bb9440e27889a364bcb678b1f679ecd1347acdedcbf36e83494f857cc58026")
                .unwrap()[..]
        );
    }

    #[test]
    fn pedersen2_test1() {
        let bytes_be = u64_to_bytes_be(0x0807060504030201);
        let a = Felt::from_bytes_be_slice(&bytes_be);

        let bytes_be = u64_to_bytes_be(0x8070605040302010);
        let b = Felt::from_bytes_be_slice(&bytes_be);

        let h = pedersen_hash(&a, &b);
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
        let a = Felt::from_bytes_be_slice(&bytes_be);

        let bytes_be = u64_to_bytes_be(0xb);
        let b = Felt::from_bytes_be_slice(&bytes_be);

        let bytes_be = u64_to_bytes_be(0xc);
        let c = Felt::from_bytes_be_slice(&bytes_be);

        let h = pedersen_array(&[a, b, c]);

        assert_eq!(
            h,
            // output taken from running ../cairo-scripts/test_pedersen_array.cairo
            felt_from_dec_str(
                "-1057847935836077748022753357540565488967807821699195068499127579478649353315"
            )
            .unwrap()
        );
    }
}
