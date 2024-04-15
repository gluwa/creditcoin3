use arbitrary::{Arbitrary, Unstructured};
use rand::Rng;
use sp_core::H256;

pub fn arbitrary_h256(u: &mut Unstructured<'_>) -> arbitrary::Result<H256> {
    u.arbitrary::<[u8; 32]>().map(Into::into)
}

/// Makes a random instance of `T`.
pub fn random<T: for<'a> Arbitrary<'a>>() -> T {
    let mut rng = rand::thread_rng();
    let (min, max) = T::size_hint(0);
    let capacity = max.unwrap_or(min * 2);
    let mut data = vec![0u8; capacity];
    let mut retries = 0;
    const MAX_RETRIES: u32 = 10;
    loop {
        rng.fill(&mut data[..]);
        let u = Unstructured::new(&data);
        match T::arbitrary_take_rest(u) {
            Ok(x) => return x,
            Err(arbitrary::Error::NotEnoughData) => {
                // Double the buffer's size. Optionally have a max
                // buffer size.
                let new_len = data.len() * 2;
                data.resize(new_len, 0);
                continue;
            }
            Err(_) => {
                // Just try again with new data. Optionally have a
                // max number of retries.
                retries += 1;
                if retries > MAX_RETRIES {
                    panic!("too many retries");
                }
                continue;
            }
        }
    }
}