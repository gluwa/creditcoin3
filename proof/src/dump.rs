use std::fmt::Debug;
use std::fs::create_dir_all;
use std::marker::PhantomData;
use std::mem::size_of;
use anyhow::anyhow;
use ethereum::{EIP1559Transaction, EIP2930Transaction, LegacyTransaction};
use ethereum_types::{Address, H160, U256};
use serde::{Deserialize, Serialize};
use starknet_crypto::{FieldElement, pedersen_hash};
use mmr::Mmr;
use mmr::proof::Proof;
use prover_primitives::claim::{Claim, ClaimKind};
use crate::Felt;

#[derive(Debug, Serialize, Deserialize)]
pub struct MerkleProofWithClaimJson {
    height: usize,
    arity: usize,
    root: String,
    path: Vec<Vec<String>>,
    claim_rlp: Vec<u8>,
    leaf_hash_prefix: u8,
    inner_node_hash_prefix: u8,
    claim_kind: ClaimKind,
}

impl From<(StarknetPedersenMerkleProof, Vec<u8>, ClaimKind)> for MerkleProofWithClaimJson {
    fn from(
        (proof, claim_rlp, claim_kind): (StarknetPedersenMerkleProof, Vec<u8>, ClaimKind),
    ) -> Self {
        Self {
            height: proof.height(),
            arity: StarknetPedersenMerkleProof::arity(),
            root: proof.root().0.to_string(),
            path: proof
                .path()
                .as_ref()
                .iter()
                .map(|item| {
                    let mut v: Vec<_> = item
                        .hashes()
                        .iter()
                        .map(|felt_wrapped| felt_wrapped.0.to_string())
                        .collect();
                    v.push(item.offset().to_string());
                    v
                })
                .collect(),
            claim_rlp,
            leaf_hash_prefix: mmr::LEAF_HASH_PREPEND_VALUE,
            inner_node_hash_prefix: mmr::INNER_HASH_PREPEND_VALUE,
            claim_kind,
        }
    }
}

#[derive(Serialize)]
pub struct ClaimCairoVerifier<'a> {
    claim_with_merkle_proof: MerkleProofWithClaimJson,
    claim_digest_roots: ClaimDigestRoots,
    attestation_chain: FragmentSliceSerializable<'a>,

    #[serde(skip_serializing, skip_deserializing)]
    claim_block_number: u64,
    #[serde(skip_serializing, skip_deserializing)]
    claim_kind: ClaimKind,
    #[serde(skip_serializing, skip_deserializing)]
    claim_index: usize,
    #[serde(skip_serializing, skip_deserializing)]
    fname: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    cairo_output_file: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    dir: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    cairo_output: Option<CairoVerifierOutput>,
}

async fn run_cairo_verify_script(
    script_source: &str,
    input_dir: &str,
    cairo_proof_mode: bool,
) -> Result<(), ScriptError> {
    use std::io::Write;

    tokio::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!(
            "source {} {} {}",
            script_source,
            input_dir,
            if cairo_proof_mode { "proof_mode" } else { "" },
        ))
        .stdout(std::process::Stdio::inherit())
        .output()
        .await
        .map_err(|_err| ScriptError::ProcessExecutionFailure)
        .and_then(|output| {
            output.status.success().then_some(()).ok_or({
                let _ = std::io::stdout().write_all(&output.stdout);
                let _ = std::io::stdout().write_all(&output.stderr);

                output.status.code().into()
            })
        })
}

async fn run_stone_prover_script(
    script_source: &str,
    input_dir: &str,
    force_stone_proving: bool,
) -> Result<String, CairoVerifierError> {
    use std::io::Write;
    let output = tokio::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!(
            "source {} {} {}",
            script_source,
            input_dir,
            if force_stone_proving { "force" } else { "" }
        ))
        .stdout(std::process::Stdio::inherit())
        .output()
        .await
        .map_err(|_err| ScriptError::ProcessExecutionFailure)?;
    if output.status.code() == Some(43) {
        return Ok("WARNING: proof file already exists, skipping stone-proving. Use force_stone_proving flag for forcing stone-proving".to_owned());
    }
    if output.status.success() {
        Ok("done".to_owned())
    } else {
        let _ = std::io::stdout().write_all(&output.stdout);
        let _ = std::io::stdout().write_all(&output.stderr);

        Err(CairoVerifierError::Cairo(output.status.code().into()))
    }
}

impl<'a> ClaimCairoVerifier<'a> {
    const SCRIPT_SOURCE: &'static str = "../cairo-scripts/verify_merkle_proof.sh";
    const STONE_PROVER_SCRIPT_SOURCE: &'static str = "../cairo-scripts/stone_prove_claim.sh";

    pub fn script_source() -> &'static str {
        Self::SCRIPT_SOURCE
    }
    pub fn stone_prover_script_source() -> &'static str {
        Self::STONE_PROVER_SCRIPT_SOURCE
    }

    pub async fn cairo_verify(
        mut self,
        cairo_proof_mode: bool,
    ) -> Result<Self, CairoVerifierError> {
        match &self.dir {
            Some(dir) => run_cairo_verify_script(Self::script_source(), dir, cairo_proof_mode)
                .await
                .map_err(|err| CairoVerifierError::Cairo(err))
                .and_then(|_| {
                    self.cairo_output = Some(self.read_output()?);
                    Ok(self)
                }),
            None => Err(CairoVerifierError::InputFileNameNotSet),
        }
    }
    pub fn take_output(&mut self) -> Option<CairoVerifierOutput> {
        self.cairo_output.take()
    }
    pub async fn stone_prove(
        self,
        force_stone_proving: bool,
    ) -> Result<String, CairoVerifierError> {
        match &self.dir {
            Some(dir) => {
                run_stone_prover_script(Self::STONE_PROVER_SCRIPT_SOURCE, dir, force_stone_proving)
                    .await
            }
            None => Err(CairoVerifierError::InputFileNameNotSet),
        }
    }
    pub fn file_name(&self) -> Option<&str> {
        self.fname.as_ref().map(String::as_str)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSerializable<'a> {
    block_number: String,
    tx_root: String,
    rx_root: String,
    prev_digest: String,
    digest: String,
    #[serde(skip_serializing, skip_deserializing)]
    _marker: PhantomData<&'a ()>,
}

impl<'a> From<&'a Block> for BlockSerializable<'a> {
    fn from(b: &'a Block) -> Self {
        Self {
            block_number: b.block_number.to_string(),
            tx_root: b.tx_root.to_string(),
            rx_root: b.rx_root.to_string(),
            prev_digest: b.prev_digest.to_string(),
            digest: b.digest.to_string(),
            _marker: PhantomData,
        }
    }
}

impl TryFrom<BlockSerializable<'_>> for Block {
    type Error = ();

    fn try_from(block: BlockSerializable) -> Result<Self, ()> {
        Ok(Self {
            block_number: block.block_number.parse().map_err(|_| ())?,
            tx_root: Felt::from_dec_str(block.tx_root.as_ref()).map_err(|_| ())?,
            rx_root: Felt::from_dec_str(block.rx_root.as_ref()).map_err(|_| ())?,
            prev_digest: Felt::from_dec_str(block.prev_digest.as_ref()).map_err(|_| ())?,
            digest: Felt::from_dec_str(block.digest.as_ref()).map_err(|_| ())?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentSliceSerializable<'a> {
    pub blocks: Vec<BlockSerializable<'a>>,
}

impl<'a> From<FragmentSlice<'a>> for FragmentSliceSerializable<'a> {
    fn from(slice: FragmentSlice<'a>) -> Self {
        Self {
            blocks: slice.0.iter().map(BlockSerializable::from).collect(),
        }
    }
}

impl<'a> ClaimCairoVerifier<'a> {
    fn new(
        merkle_proof: StarknetPedersenMerkleProof,
        rlp: Vec<u8>,
        claim_block_number: u64,
        claim_kind: ClaimKind,
        claim_index: usize,
        claim_digest_roots: ClaimDigestRoots,
        attestation_chain: FragmentSliceSerializable<'a>,
    ) -> Self {
        Self {
            claim_with_merkle_proof: (merkle_proof, rlp, claim_kind).into(),
            claim_block_number,
            claim_kind,
            claim_index,
            claim_digest_roots,
            attestation_chain,
            fname: None,
            dir: None,
            cairo_output_file: None,
            cairo_output: None,
        }
    }

    fn with_default_files(mut self) -> anyhow::Result<Self> {
        let dir = self.default_dir(self.claim_block_number, self.claim_kind, self.claim_index);

        create_dir_all(&dir)?;

        let cairo_input_file = Self::default_cairo_input_file_name(&dir);
        let cairo_output_file = Self::default_cairo_output_file_name(&dir);

        self.to_file(&cairo_input_file)?;

        self.dir = Some(dir);
        self.fname = Some(cairo_input_file);
        self.cairo_output_file = Some(cairo_output_file);
        Ok(self)
    }

    fn to_file(&self, fname: &str) -> anyhow::Result<()> {
        use std::fs::File;
        use std::io::{BufWriter, Write};

        let file = File::create(fname)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)?;
        Ok(writer.flush()?)
    }

    fn default_dir(&self, block_number: u64, claim_kind: ClaimKind, claim_index: usize) -> String {
        let hex_block_number = format!("0x{:X}", block_number);

        let partial_dir = &format!(
            "block_{hex_block_number}/{}{claim_index}",
            claim_kind.subdir()
        );
        //format!("{}/{partial_dir}", claim_proof_dir())
    }

    fn default_cairo_input_file_name(dir: &str) -> String {
        format!("{dir}/program_input.json")
    }
    fn default_cairo_output_file_name(dir: &str) -> String {
        format!("{dir}/output.txt")
    }
    fn read_output(&self) -> Result<CairoVerifierOutput, CairoVerifierError> {
        self.cairo_output_file
            .as_ref()
            .ok_or(CairoVerifierError::OutputFileNameNotSet)
            .and_then(|cairo_output_file| {
                let output_str = std::fs::read_to_string(cairo_output_file)
                    .map_err(|err| CairoVerifierError::OutputParseFailure(format!("{err:?}")))?;

                CairoVerifierOutput::try_from(&output_str[..])
                    .map_err(|err| CairoVerifierError::OutputParseFailure(format!("{err:?}")))
            })
    }
}

#[derive(Debug, Clone)]
pub struct FragmentSlice<'a>(&'a [Block]);

#[derive(Debug, Clone, Default)]
pub struct Block {
    block_number: u64,
    tx_root: Felt,
    rx_root: Felt,
    prev_digest: Felt,
    digest: Felt,
}

pub trait BlockItem: Sized {
    fn to_bytes(&self) -> Vec<u8>;

    fn chain_id(&self) -> u64;
    fn block_number(&self) -> U256;
    fn index(&self) -> u64;
    fn from(&self) -> Address;
    fn to(&self) -> Option<Address>;
}

pub trait FetchFromBlock: Sized {
    type Cache: CacheT<Self>;
    type ErrorType: Debug;

    fn fetch_all(
        url: &str,
        cache: Option<&mut Self::Cache>,
        block_number: u64,
    ) -> impl std::future::Future<Output = Result<Vec<Self>, Self::ErrorType>> + Send;

    fn fetch_from_cache(cache: &Self::Cache) -> Result<Vec<Self>, Self::ErrorType>;
}

pub trait CacheT<T>: Clone {
    type CachedItem: TryInto<T> + Serialize + for<'a> Deserialize<'a>;

    fn key(&self) -> &str;
    fn try_create_key(&mut self) -> anyhow::Result<()>;

    fn try_read(&self) -> anyhow::Result<Vec<Self::CachedItem>> {
        let file = std::fs::File::open(self.key())?;
        Ok(serde_json::from_reader::<_, Vec<Self::CachedItem>>(file)?)
    }

    fn try_write(&mut self, items: &[Self::CachedItem]) -> anyhow::Result<()> {
        self.try_create_key()?;

        let file = std::fs::File::create(self.key())?;

        Ok(serde_json::to_writer(file, items)?)
    }
}

#[derive(Debug)]
pub enum TypedTransaction {
    Type0(LegacyTransaction, TransactionExtension),
    Type1(EIP2930Transaction, TransactionExtension),
    Type2(EIP1559Transaction, TransactionExtension),
}

#[derive(Debug)]
pub struct TransactionExtension {
    chain_id: u64,
    block_number: U256,
    index: u64,
    from: Address,
    to: Option<Address>,
    prefix: Option<u8>,
}

impl TransactionExtension {
    pub fn to_bytes(&self) -> Vec<u8> {
        TransactionClaimProverPublicInput::from(self).to_bytes()
    }
}

#[repr(C)]
#[derive(Default)]
struct TransactionClaimProverPublicInput {
    chain_id: [u8; size_of::<u64>()],
    block_number: [u8; size_of::<U256>()],
    index: [u8; size_of::<u64>()],
    from: [u8; size_of::<Address>()],
    fence1: u8,
    to: Option<[u8; size_of::<Address>()]>,
    prefix: Option<u8>,
}

impl TransactionClaimProverPublicInput {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(size_of::<Self>());

        v.extend(self.chain_id);
        v.extend(self.block_number);
        v.extend(self.index);
        v.extend(self.from);
        v.push(self.fence1);
        if let Some(to) = self.to.as_ref() {
            v.extend(to);
        }
        v.extend(self.prefix.as_slice());

        v
    }
}
impl From<&TransactionExtension> for TransactionClaimProverPublicInput {
    fn from(transaction_extension: &TransactionExtension) -> Self {
        let mut this = Self {
            chain_id: transaction_extension.chain_id.to_be_bytes(),
            index: transaction_extension.index.to_be_bytes(),
            from: transaction_extension.from.to_fixed_bytes(),
            fence1: transaction_extension.to.is_some().into(),
            to: transaction_extension.to.map(Address::to_fixed_bytes),
            prefix: transaction_extension.prefix,
            ..Default::default()
        };
        transaction_extension
            .block_number
            .to_big_endian(&mut this.block_number);
        this
    }
}

pub fn block_cache_dir() -> String {
    format!("{}/{}", DATA_ROOT_DIR, BLOCK_CACHE_DIR)
}

const DATA_ROOT_DIR: &str = "../data";
const BLOCK_CACHE_DIR: &str = "block-cache";

#[derive(Debug)]
pub enum SortedBlockError {
    NotFound,
    NotUnique,
    FetchFailure(String),
}

pub struct SortedBlock<T: BlockItem + FetchFromBlock>(Vec<T>);

impl<T: BlockItem + FetchFromBlock> SortedBlock<T> {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn iter(&self) -> <&Vec<T> as IntoIterator>::IntoIter {
        self.into_iter()
    }
    pub fn find_all<P: Fn(&&T) -> bool>(&self, predicate: P) -> Option<Vec<usize>> {
        self.iter().find_all(predicate)
    }
    pub fn find_unique<P: FnMut(&&mut T) -> bool>(
        &mut self,
        mut predicate: P,
    ) -> Result<&mut T, SortedBlockError> {
        let mut it = self.0.iter_mut();

        let item = it.find(&mut predicate).ok_or(SortedBlockError::NotFound)?;
        it.find(predicate)
            .map_or(Ok(item), |_| Err(SortedBlockError::NotUnique))
    }
    pub fn find_claim(&mut self, claim: &Claim<T>) -> Result<&mut T, SortedBlockError> {
        self.find_unique(|item| {
            item.block_number() == claim.block_number.into()
                && claim
                .chain_id
                .map(|chain_id| chain_id.eq(&item.chain_id()))
                .unwrap_or(true)
                && claim
                .tx_index.eq(&item.index())
                // .map(|index| index.eq(&item.index()))
                // .unwrap_or(true)
                && claim.from.map(|from| from.eq(&item.from())).unwrap_or(true)
                && claim.to.map(|to| Some(to).eq(&item.to())).unwrap_or(true)
        })
    }
    pub fn to_bytes(&self) -> Vec<Vec<u8>> {
        self.into_iter().map(T::to_bytes).collect()
    }
    pub async fn try_fetch(
        url: &str,
        cache: Option<&mut T::Cache>,
        block_number: u64,
    ) -> Result<Self, SortedBlockError> {
        T::fetch_all(url, cache, block_number)
            .await
            .map(Self::from)
            .map_err(|err| SortedBlockError::FetchFailure(format!("{err:?}")))
    }
}

impl<V, T> From<V> for SortedBlock<T>
    where
        V: IntoIterator<Item = T>,
        T: BlockItem + FetchFromBlock,
{
    fn from(v: V) -> Self {
        Self::from_iter(v)
    }
}

impl<T: BlockItem + FetchFromBlock> FromIterator<T> for SortedBlock<T> {
    fn from_iter<V: IntoIterator<Item = T>>(it: V) -> Self {
        let mut v = Vec::<_>::from_iter(it);
        v.sort_by_key(T::index);

        Self(v)
    }
}

impl<'a, T: BlockItem + FetchFromBlock> IntoIterator for &'a SortedBlock<T> {
    type Item = &'a T;
    type IntoIter = <&'a Vec<T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> <Self as IntoIterator>::IntoIter {
        self.0.iter()
    }
}

pub async fn build_verifier<'a>(
    url: &str,
    claim: Claim<H160>,
    attestation_chain_slice: FragmentSlice<'a>,
) -> Result<ClaimCairoVerifier<'a>, CairoVerifierError> {
    let claim_block_number: u64 = claim.block_number;

    let tx_cache = &mut <TypedTransaction as FetchFromBlock>::Cache::new(
        &block_cache_dir(),
        claim_block_number,
    );
    let fetch_tx_block_fut =
        SortedBlock::<TypedTransaction>::try_fetch(url, Some(tx_cache), claim_block_number);

    let rx_cache =
        &mut <Receipt as FetchFromBlock>::Cache::new(&block_cache_dir(), claim_block_number);
    let fetch_rx_block_fut =
        SortedBlock::<Receipt>::try_fetch(url, Some(rx_cache), claim_block_number);

    let (mut sorted_transactions_block, mut sorted_receipts_block) =
        futures::future::try_join(fetch_tx_block_fut, fetch_rx_block_fut).await?;

    let claim_index = match claim.kind {
        ClaimKind::Tx => sorted_transactions_block.find_claim(&claim)?.index(),
        ClaimKind::Rx => sorted_receipts_block.find_claim(&claim)?.index(),
    };

    let tx_bytes = sorted_transactions_block.to_bytes();
    let rx_bytes = sorted_receipts_block.to_bytes();

    let (transactions_tree, receipts_tree) =
        futures::future::join(async { StarknetPedersenMmr::from(&tx_bytes[..]) }, async {
            StarknetPedersenMmr::from(&rx_bytes[..])
        })
            .await;

    let (claim_bytes, merkle_path) = match claim.kind {
        ClaimKind::Tx => (
            tx_bytes[claim_index as usize].clone(),
            transactions_tree.generate_proof(claim_index as usize),
        ),
        ClaimKind::Rx => (
            rx_bytes[claim_index as usize].clone(),
            receipts_tree.generate_proof(claim_index as usize),
        ),
    };
    let digest_roots = ClaimDigestRoots::new(&transactions_tree.root().0, &receipts_tree.root().0);

    let instance = ClaimCairoVerifier::new(
        merkle_path,
        claim_bytes,
        claim_block_number,
        claim.kind,
        claim_index as usize,
        digest_roots,
        attestation_chain_slice.into(),
    )
        .with_default_files()
        .map_err(|err| CairoVerifierError::SerializationFailure(format!("{err:?}")))?;

    Ok(instance)
}

pub type StarknetPedersenMmr = Mmr<StarknetPedersenHash>;
pub type StarknetPedersenMerkleProof = Proof<StarknetPedersenHash>;

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

const U248_BYTE_COUNT: usize = 31;

pub fn felts_from_bytes(bytes: &[u8]) -> Vec<FieldElement> {
    let chunks = bytes.chunks(U248_BYTE_COUNT);

    chunks
        .map(|chunk| {
            FieldElement::from_byte_slice_be(chunk)
                .expect("chunk length matches canonical length. qed")
        })
        .collect::<Vec<_>>()
}

pub fn pedersen_array<T: AsRef<FieldElement>>(felts: &[T]) -> FieldElement {
    let maybe_zero_prefix = *felts[0].as_ref();
    let mut prev = maybe_zero_prefix;

    //    println!("zero: {}", prev.to_string());

    for felt in &felts[1..] {
        //        println!("felt: {}", felt.as_ref().to_string());
        prev = pedersen_hash(&prev, felt.as_ref());
    }

    let len_felt = FieldElement::from_byte_slice_be(&u64_to_bytes_be((felts.len() - 1) as u64))
        .expect("length is less than canonical length. qed");

    //    println!("len: {}", len_felt.as_ref().to_string());
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

#[derive(Debug)]
pub enum CairoVerifierError {
    AttestationFragmentMismatch(u64, Option<u64>, Option<u64>),
    SerializationFailure(String),
    BlockFetchFailure(String),
    ClaimNotIdentified,
    ClaimNotUnique,
    InputFileNameNotSet,
    OutputFileNameNotSet,
    OutputParseFailure(String),
    Cairo(ScriptError),
}

#[derive(Debug)]
pub enum ScriptError {
    BadArgs,
    InputFiles(i32),
    Compilation(i32),
    Run(i32),
    StoneProver(i32),
    StoneVerifier(i32),
    AttestationProgramCompilation(i32),
    Other(i32),
    ProcessExecutionFailure,
    Unspecified,
}

impl From<Option<i32>> for ScriptError {
    fn from(code: Option<i32>) -> Self {
        if let Some(code) = code {
            match code {
                10..=19 => Self::BadArgs,
                20..=29 => Self::InputFiles(code),
                30..=39 => Self::Compilation(code),
                40..=49 => Self::Run(code),
                50..=59 => Self::StoneProver(code),
                60..=69 => Self::StoneVerifier(code),
                70 => Self::AttestationProgramCompilation(code),
                _ => Self::Other(code),
            }
        } else {
            Self::Unspecified
        }
    }
}

#[derive(Serialize)]
pub struct ClaimDigestRoots {
    tx_root: String,
    rx_root: String,
}

impl ClaimDigestRoots {
    pub fn new(tx_root: &Felt, rx_root: &Felt) -> Self {
        Self {
            tx_root: tx_root.to_string(),
            rx_root: rx_root.to_string(),
        }
    }
}

#[derive(Debug)]
pub struct CairoVerifierOutput {
    pub claim_proof_root: Felt,
    pub claim_kind: ClaimKind,
    pub claim_index: u64,
    pub block_number: u64,
    pub chain_id: u8,
    pub claim_from: Felt,
    pub claim_to: Felt,
    pub continuity_checkpoint_digest: Felt,
    pub continuity_checkpoint_block_number: u64,
}

impl CairoVerifierOutput {
    const PREFIX: &'static str = "Program output:";
}

impl TryFrom<&str> for CairoVerifierOutput {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, String> {
        if s.len() < Self::PREFIX.len() {
            return Err("failed to parse output string".to_owned());
        }

        let mut prefix_index = None;
        for i in 0..(s.len() - Self::PREFIX.len()) {
            if &s[i..i + Self::PREFIX.len()] == Self::PREFIX {
                prefix_index = Some(i + Self::PREFIX.len());
                break;
            }
        }

        if let Some(prefix_index) = prefix_index {
            let mut values_iter = s[prefix_index..].split_whitespace();

            Ok(Self {
                claim_proof_root: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'claim_proof_root' is absent".to_owned())?,
                )
                    .map_err(|err| format!("failed to parse 'claim_proof_root': {err:?}"))?,

                claim_kind: values_iter
                    .next()
                    .ok_or("value for 'claim_kind' is absent".to_owned())?
                    .parse::<u8>()
                    .map_err(|err| format!("failed to parse 'claim_kind': {err:?}"))?
                    .try_into()
                    .map_err(|err| format!("'claim_kind': {err:?}"))?,

                claim_index: values_iter
                    .next()
                    .ok_or("value for 'claim_index' is absent".to_owned())?
                    .parse::<u64>()
                    .map_err(|err| format!("failed to parse 'claim_index': {err:?}"))?,
                block_number: values_iter
                    .next()
                    .ok_or("value for 'block_number' is absent".to_owned())?
                    .parse::<u64>()
                    .map_err(|err| format!("failed to parse 'block_number': {err:?}"))?,
                chain_id: values_iter
                    .next()
                    .ok_or("value for 'chain_id' is absent".to_owned())?
                    .parse::<u8>()
                    .map_err(|err| format!("failed to parse 'chain_id': {err:?}"))?,
                claim_from: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'claim_from' is absent".to_owned())?,
                )
                    .map_err(|err| format!("failed to parse 'claim_from': {err:?}"))?,
                claim_to: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'claim_to' is absent".to_owned())?,
                )
                    .map_err(|err| format!("failed to parse 'claim_to': {err:?}"))?,
                continuity_checkpoint_digest: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'continuity_checkpoint_digest' is absent".to_owned())?,
                )
                    .map_err(|err| {
                        format!("failed to parse 'continuity_checkpoint_digest': {err:?}")
                    })?,
                continuity_checkpoint_block_number: values_iter
                    .next()
                    .ok_or("value for 'continuity_checkpoint_block_number' is absent")?
                    .parse::<u64>()
                    .map_err(|err| {
                        format!("failed to parse 'continuity_checkpoint_block_number': {err:?}")
                    })?,
            })
        } else {
            Err(format!(
                "failed to parse output string. Expected to find '{}' prefix",
                Self::PREFIX
            ))
        }
    }
}

#[allow(dead_code)]
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
