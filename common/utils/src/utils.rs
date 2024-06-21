use core::mem::size_of;

use crate::Felt;
use anyhow::anyhow;
use ethereum_types::{Address, H256, U256};

extern crate alloc;
use alloc::string::String;
use sp_std::vec::Vec;

pub const U248_BYTE_COUNT: usize = 31;
const HASH_LENGTH: usize = 32;
const HASH_LENGTH_MINUS_1: usize = HASH_LENGTH - 1;

pub fn decode_prefixed_hex(hex: &mut str) -> anyhow::Result<Vec<u8>> {
    let stripped = strip_hex_prefix(hex);
    //    let stripped = hex.trim_start_matches("0x");
    if !stripped.is_empty() {
        Ok(hex::decode(stripped).map_err(|_| anyhow!("Failed to parse hex"))?)
    } else {
        Ok(vec![0])
    }
}

pub fn strip_hex_prefix(prefixed_hex: &mut str) -> &str {
    if prefixed_hex.is_empty() {
        ""
    } else if prefixed_hex.len() % 2 == 1 {
        unsafe { prefixed_hex.as_bytes_mut()[1] = b'0' };
        &prefixed_hex[1..]
    } else {
        &prefixed_hex[2..]
    }
}

pub fn hex_to_address(hex: &mut str) -> anyhow::Result<Address> {
    decode_prefixed_hex(hex).map(|bytes| Address::from_slice(&bytes))
}

pub fn hex_to_u256(hex: &mut str) -> anyhow::Result<U256> {
    decode_prefixed_hex(hex).map(|bytes| U256::from_big_endian(&bytes))
}

// pub fn u256_to_hex(val: &U256) -> String {
//     format!("0x{:X}", val)
// }

pub fn hex_to_u64(hex: &mut str) -> anyhow::Result<u64> {
    let stripped = strip_hex_prefix(hex);
    //    let stripped = hex.trim_start_matches("0x");

    if !stripped.is_empty() {
        Ok(u64::from_str_radix(stripped, 16).map_err(|_| anyhow!("Failed to parse hex"))?)
    } else {
        Ok(0u64)
    }
}

pub fn hex_to_h256(hex: &mut str) -> anyhow::Result<H256> {
    decode_prefixed_hex(hex).and_then(|bytes| bytes_to_h256(&bytes[..]))
}

pub fn hex_strings_to_h256s(hex_strings: Vec<String>) -> anyhow::Result<Vec<H256>> {
    hex_strings
        .into_iter()
        .map(|mut s| hex_to_h256(&mut s))
        .collect()
}

fn bytes_to_h256(bytes: &[u8]) -> anyhow::Result<H256> {
    let len = bytes.len();

    match len {
        0..=HASH_LENGTH_MINUS_1 => {
            let mut buf = [0u8; HASH_LENGTH];
            buf[HASH_LENGTH - len..].copy_from_slice(bytes);
            Ok(H256::from_slice(&buf))
        }
        HASH_LENGTH => Ok(H256::from_slice(bytes)),
        _ => Err(anyhow!("H256 bytes length mismatch")),
    }
}

pub fn felt_from_dec_str(s: &str) -> anyhow::Result<Felt> {
    match Felt::from_dec_str(s) {
        Ok(x) => Ok(x),
        Err(_) if s.starts_with('-') => {
            let neg_x = Felt::from_dec_str(&s[1..]).map_err(|err| anyhow!("{}", err))?;
            Ok(Felt::ZERO - neg_x)
        }
        Err(err) => Err(anyhow!("{}", err)),
    }
}

pub fn try_parse_usize(s: &str) -> Result<usize, core::num::ParseIntError> {
    s.parse::<usize>()
        .or_else(|_| usize::from_str_radix(s.trim_start_matches("0x"), 16))
}
pub fn try_parse_u64(s: &str) -> Result<u64, core::num::ParseIntError> {
    //    println!("S: {:?}", Felt::from_dec_str(s));
    s.parse::<u64>()
        .or_else(|_| u64::from_str_radix(s.trim_start_matches("0x"), 16))
}
pub fn try_parse_felt(s: &str) -> Result<Felt, starknet_ff::FromStrError> {
    felt_from_dec_str(s).or_else(|_| Felt::from_hex_be(s))
}

pub fn address_from_felt(felt: &Felt) -> Address {
    Address::from_slice(&felt.to_bytes_be()[size_of::<Felt>() - size_of::<Address>()..])
}

pub fn felts_from_bytes(bytes: &[u8]) -> Vec<Felt> {
    let chunks = bytes.chunks(U248_BYTE_COUNT);

    chunks
        .map(|chunk| {
            Felt::from_byte_slice_be(chunk)
                .expect("chunk length doesn't exceed canonical length. qed")
        })
        .collect::<Vec<_>>()
}

// converts felt array to byte array
// felt array is assumed to be formed from 31-byte long chunks using Felt::from_byte_slice_be
// source_bytes_len is needed to be provided in order the conversion to yield the same
// source byte array used to form the felt array
// if source_bytes_len is not provided the resulting array tail may have zero padding not present in the original
// byte array
pub fn felts_to_bytes(felts: &[Felt], source_bytes_len: Option<usize>) -> Vec<u8> {
    const U248_OFFSET: usize = 32 - U248_BYTE_COUNT; // U248_OFFSET == 1

    let mut bytes = Vec::with_capacity(source_bytes_len.unwrap_or(U248_BYTE_COUNT * felts.len()));

    felts
        .iter()
        // take all but the last felt
        .rev()
        .skip(1)
        .rev()
        .for_each(|felt| bytes.extend(&felt.to_bytes_be()[U248_OFFSET..]));

    // need to shift last 32 (be) bytes left according to the source length
    // the last_offset must be in range [1, 31] since U248_BYTE_COUNT == 31
    let last_offset = source_bytes_len
        // will yield values in range [2, 32]
        // 32 must yet be mapped to 1
        .map(|len| 32 - len % U248_BYTE_COUNT)
        // maps 32 => 1
        .map(|x| x - U248_BYTE_COUNT * (x / 32))
        // if source byte length in not provided assume offset 1
        .unwrap_or(1);

    if let Some(last) = felts.last() {
        bytes.extend(&last.to_bytes_be()[last_offset..]);
    }

    bytes
}

pub fn u256_from_felts(lo: &Felt, hi: &Felt) -> U256 {
    let mut buf = lo.to_bytes_be();
    buf[0] = hi.to_bytes_be()[31];

    U256::from_big_endian(&buf[..])
}

pub fn u256_to_felts(x: &U256) -> (Felt, Felt) {
    let mut buf = [0u8; 32];
    x.to_big_endian(&mut buf);
    let lo = Felt::from_byte_slice_be(&buf[1..32]).expect("less that 256 bits");
    let hi = Felt::from(buf[0]);

    // let mut buf_hi = [0u8; 31];
    // buf_hi[30] = buf[0];
    // let hi = Felt::from_byte_slice_be(&buf_hi[..]).expect("less that 256 bits");

    (lo, hi)
}

pub fn h256_from_felts(lo: &Felt, hi: &Felt) -> H256 {
    let mut buf = lo.to_bytes_be();
    buf[0] = hi.to_bytes_be()[31];

    H256::from_slice(&buf[..])
}

pub fn h256_to_felts(x: &H256) -> (Felt, Felt) {
    let buf = x.to_fixed_bytes();
    let lo = Felt::from_byte_slice_be(&buf[1..32]).expect("less that 256 bits");
    let hi = Felt::from(buf[0]);

    (lo, hi)
}

#[cfg(test)]
mod tests {
    use crate::utils::{
        felts_from_bytes, felts_to_bytes, h256_from_felts, h256_to_felts, u256_from_felts,
        u256_to_felts,
    };
    use ethereum_types::{H256, U256};

    extern crate alloc;
    use alloc::vec;
    use alloc::vec::Vec;
    use libc_print::std_name::println;

    #[test]
    fn felt_to_bytes_test() {
        for i in 0..3333 {
            let v = (0..i as u8).into_iter().collect::<Vec<u8>>();
            let felts = felts_from_bytes(&v[..]);

            let bytes = felts_to_bytes(&felts[..], Some(v.len()));
            assert_eq!(v, bytes);
        }
        for i in 0..1025 {
            let v = vec![0u8; i];
            let felts = felts_from_bytes(&v[..]);

            let bytes = felts_to_bytes(&felts[..], Some(v.len()));
            assert_eq!(v, bytes);
        }
    }

    #[test]
    fn felts_u256_conversion_test() {
        let bytes = (33..65).into_iter().collect::<Vec<u8>>();
        let u256 = U256::from_big_endian(&bytes[..]);
        //        let u256: U256 = 1.into();

        println!("u256: {}", u256);
        let (lo, hi) = u256_to_felts(&u256);
        println!("lo: {lo:?}, hi: {hi:?}");
        let expected = u256_from_felts(&lo, &hi);
        assert_eq!(u256, expected);
    }

    #[test]
    fn felts_h256_conversion_test() {
        let bytes = (33..65).into_iter().collect::<Vec<u8>>();
        let h256 = H256::from_slice(&bytes[..]);
        //        let u256: U256 = 1.into();

        println!("h256: {}", h256);
        let (lo, hi) = h256_to_felts(&h256);
        println!("lo: {lo:?}, hi: {hi:?}");
        let expected = h256_from_felts(&lo, &hi);
        assert_eq!(h256, expected);
    }
}
