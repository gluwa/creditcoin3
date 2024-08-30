%builtins output pedersen range_check bitwise

from starkware.cairo.common.alloc import alloc
from starkware.cairo.common.cairo_builtins import HashBuiltin
from starkware.cairo.common.hash import hash2
from starkware.cairo.common.math import assert_nn

const PADDING_SIZE = 1;
const INDEX_SIZE = 1;
const INDEX_FELT_OFFSET = 1;
const NULL_LEAF_HASH = 0;


struct AttestationBlock {
    block_number: felt,
    merkle_root: felt,
    prev_digest: felt,
    digest: felt,
}
func pedersen_hash2{pedersen_ptr: HashBuiltin*, range_check_ptr}(x, y) -> felt {
    let (res) = hash2{hash_ptr=pedersen_ptr}(x, y);
    return res;
}

func pedersen_array{pedersen_ptr: HashBuiltin*, range_check_ptr}(arr: felt*, array_len) -> felt {
    assert_nn(array_len - 2);

    if (array_len == 2) {
        return pedersen_hash2(arr[0], arr[1]);
    }

    let h = pedersen_array(arr=arr, array_len=array_len - 1);
    
    return pedersen_hash2(h, arr[array_len - 1]);
}

// verifies the Merkle proof and return the index of the claim as a leaf of the Merkle tree
// that generated the proof
func verify_merkle_path{output_ptr, pedersen_ptr: HashBuiltin*, range_check_ptr}(
    root,
    word_hash, 
    proof_items: felt*, 
    offsets: felt*, 
    arity, 
    height, 
    i,
    ind,
    a,
    zero_height_hamming_distance,
    inner_node_prefix
//) {
) -> (felt, felt) {
    // case of empty block, return index 0 as out-of-bounds witness
    if (height == 0) {
        return (0, 0);
    } 
    let h = hash_with_siblings(
                word_hash, 
                proof_item=proof_items, 
                offset=offsets[0], 
                arity=arity, 
                inner_node_prefix=inner_node_prefix
            );

    // add exactly 1 to zero_height_hamming_distance iff current offset == arity - 1,
    // this recursive function output zero_height_hamming_distance == proof_height
    // iff this condition holds for every iteration 
    let zero_height_hamming_distance = zero_height_hamming_distance + (arity - offsets[0]);

    if (i == height - 1) {
        // check if the last hash and the root are identical
        assert root = h;
        return (ind + a * offsets[0], zero_height_hamming_distance);
    }
    // calculate claim index from proof's offsets
    let index = ind + a * offsets[0];
    let a = a * arity;

    return verify_merkle_path(
                root, 
                h, 
                proof_items=proof_items+arity, 
                offsets=offsets+1, 
                arity=arity, 
                height=height, 
                i=i+1, 
                ind=index, 
                a=a,
                zero_height_hamming_distance=zero_height_hamming_distance,
                inner_node_prefix=inner_node_prefix
            );
}

func hash_with_siblings{output_ptr, pedersen_ptr: HashBuiltin*, range_check_ptr}(
    word_hash, 
    proof_item: felt*, 
    offset, 
    arity,
    inner_node_prefix
) -> felt {
    alloc_locals;
    let (curr_item) = alloc();

    init_curr_item(
            item=curr_item, 
            word=word_hash, 
            original=proof_item, 
            at=offset, 
            len=arity, 
            i=0, 
            inner_node_prefix=inner_node_prefix,
        );

    let h = pedersen_array(arr=curr_item, array_len=arity + 2);

    return h;
}

// takes the current original proof item and copies it to an array pointed by "item"
// at all locations except that at index "at", where the original felt is replaced by the "word"
func init_curr_item{range_check_ptr}(item: felt*, word, original: felt*, at, len, i, inner_node_prefix) {
    alloc_locals;
    assert_nn(len - at - 1);

    // put arity(number of felts in item) as last element
    if (len == 0) {
        assert [item + i] = i;
        return ();
    }
    // put zero as first element
    if (i == 0) {
        //assert [item + i] = 0;
        assert [item + i] = inner_node_prefix;
        init_curr_item(
                item=item, 
                word=word, 
                original=original, 
                at=at, 
                len=len, 
                i=i+1, 
                inner_node_prefix=inner_node_prefix
            );
        return();
    }

    if (at == 0) {
        // replace original felt with claim's felt
        assert [item + i] = word;
    } else {
        // copy original felt
        assert [item + i] = original[0];
    }
    
    init_curr_item(
            item=item, 
            word=word, 
            original=original+1, 
            at=at-1, 
            len=len-1, 
            i=i+1, 
            inner_node_prefix=inner_node_prefix
        );
    return ();
}

// test initialization of a path item with claim hash
// auxiliary - for dev & debugging
func test_init_curr_item{range_check_ptr}(proof_felts: felt*, at, proof_arity) {
    alloc_locals;
    let (curr_item) = alloc();

    let claim = 42;
    let zero_prepend_offset = 1;
    let inner_node_prefix = 1;

    init_curr_item(
        item=curr_item, 
        word=claim, 
        original=proof_felts, 
        at=at, 
        len=proof_arity, 
        i=0, 
        inner_node_prefix=inner_node_prefix
    );

    assert curr_item[0] = 0;
    assert curr_item[zero_prepend_offset + at] = claim;
    assert curr_item[zero_prepend_offset + proof_arity] = proof_arity;

    test_rest_item_intact(proof_felts, curr_item, at, proof_arity, i=zero_prepend_offset);
    
    return();
}

// auxiliary - for dev & debugging
func test_rest_item_intact{range_check_ptr}(proof_felts: felt*, item: felt*, at, proof_arity, i) {
    let zero_prepend_offset = 1;

    if (i == proof_arity + zero_prepend_offset) {
        return ();
    }

    if (i - zero_prepend_offset != at) {
        assert proof_felts[i - zero_prepend_offset] = item[i];
    }
    test_rest_item_intact(proof_felts, item, at, proof_arity, i=i+1);
    return ();
}

func generate_continuity_attestation{pedersen_ptr: HashBuiltin*, range_check_ptr}(attestation_blocks: AttestationBlock*, len) -> AttestationBlock {
    assert_nn(len - 1);

    let d = pedersen_hash2(attestation_blocks[0].block_number, attestation_blocks[0].merkle_root);
//    let d2 = pedersen_hash2(d1, attestation_blocks[0].rx_root);
    let digest = pedersen_hash2(d, attestation_blocks[0].digest);

    if (len == 1) {
        return attestation_blocks[0];
    }

    return generate_continuity_attestation(attestation_blocks=attestation_blocks+AttestationBlock.SIZE, len=len-1);
}

// outputs an array of felts: the last felt output is the array's lenght
func output_array_at_offsets{output_ptr}(arr: felt*, offsets: felt*, offsets_len: felt) {
    return output_array_at_offsets_aux(arr=arr, ind=0, offsets=offsets, offsets_len=offsets_len);
}

func output_array_at_offsets_aux{output_ptr}(arr: felt*, ind, offsets: felt*, offsets_len) {
    if (ind == offsets_len) {
        assert [output_ptr] = offsets_len;
        let output_ptr = output_ptr + 1;

        return ();
    }

    assert [output_ptr] = arr[offsets[ind]];
    let output_ptr = output_ptr + 1;

    return output_array_at_offsets_aux(arr=arr, ind=ind+1, offsets=offsets, offsets_len=offsets_len);
}

func main{output_ptr, pedersen_ptr: HashBuiltin*, range_check_ptr, bitwise_ptr}() {
    alloc_locals;

    local proof_height;
    local proof_arity;
    local proof_root;
    local proof_felts: felt*;
    local proof_offsets: felt*;
    
    local prefixed_subject_felts: felt*;
    local prefixed_subject_felts_len;
    
    local query_offsets: felt*;
    local query_offsets_len;
    
    local inner_node_prefix;

    local block_number;
    local claim_index;
    local block_number;
    local claim_index;

    local digest_root_to_match_to_claim_root;
    local digest;
    // local digest_tx_root;
    // local digest_rx_root;
    local prev_block_digest;
    local curr_digest_from_attestation_chain;

    local attestation_blocks: AttestationBlock*;
    local attestation_blocks_len;

    local claim_leaf_hash;
    local out_of_bounds_flag;

    local claim_leaf_hash;
    local out_of_bounds_flag;
    // parse the claim and assign local variables
    %{ 
        # parse bytes as 248-bit long elements (closest to canonical safe representation of felts)
        FELT_SIZE = 31
        def flatten_list(nested_list):
            return [item for sublist in nested_list for item in sublist]
        # convert bytes to felts as big endian
        def bytes_to_felt_array(bytes):
            return [int.from_bytes(bytes[i : i + FELT_SIZE], "big") for i in range(0, len(bytes), FELT_SIZE)]

        block_number = int(program_input['block_number'])
        ids.block_number = block_number

        merkle_proof = program_input['merkle_proof']
        claim = program_input['claim']
        digest_roots = program_input['claim_digest_roots']
        attestation_chain = program_input['attestation_chain']['blocks']

        ids.out_of_bounds_flag = int(program_input['out_of_bounds_flag'])

        #ids.claim_id.claim_kind = 1 if claim['id']['kind'] == "Tx" else 2
        #claim_kind = 1 if claim['id']['kind'] == "Tx" else 2
        
        #ids.digest_root_to_match_to_claim_root = int(digest_roots['tx_root'] if claim_kind == 1 else digest_roots['rx_root'])
        ids.digest_root_to_match_to_claim_root = int(digest_roots['merkle_root'])
        #ids.digest_tx_root = int(digest_roots['tx_root'])
        #ids.digest_rx_root = int(digest_roots['rx_root'])

        ids.proof_root = int(merkle_proof['root'])
        ids.proof_height = merkle_proof['height'] 
        ids.proof_arity = arity = merkle_proof['arity'] 
        ids.inner_node_prefix = int(merkle_proof['inner_node_hash_prefix'])

        path = merkle_proof['path']
        flat_path = [int(felt) for felt in flatten_list(path)]

        ids.proof_felts = path_felts_ind = segments.add()
        ids.proof_offsets = path_offsets_ind = segments.add()
        
        leaf_offset = -1
        felts = list()
        for i, felt_or_offset in enumerate(flat_path):
            if (i + 1) % (arity + 1) != 0:
                felts.append(felt_or_offset)
                memory[path_felts_ind] = felt_or_offset; path_felts_ind += 1
            else:
                if leaf_offset == -1:
                    leaf_offset = felt_or_offset
                memory[path_offsets_ind] = felt_or_offset; path_offsets_ind += 1

        # claim leaf is undefined for empty block, so set it to 0
        ids.claim_leaf_hash = felts[leaf_offset] if leaf_offset >= 0 else 0 

        subject_bytes_prefixed = [merkle_proof['leaf_hash_prefix']] + merkle_proof['claim_subject']
        prefixed_subject_felts = bytes_to_felt_array(subject_bytes_prefixed)

        # prefixed_subject_felts will be hashed as a Merkle leaf
        ids.prefixed_subject_felts = ind = segments.add()
        length = len(prefixed_subject_felts)
        for i in range(0, length):
            memory[ind] = prefixed_subject_felts[i]; ind += 1
        # the last element contains the length of the claim without the prepended item
        memory[ind] = length
        # total length of the claim felt array plus the last item containing (ids.prefixed_subject_felts_len - 1)
        ids.prefixed_subject_felts_len = length + 1

        # refer to BlockItemIdentifier::to_bytes() padding in Rust lib
        indexOffset = FELT_SIZE
        # blockNumberOffset = FELT_SIZE
        #block_number = int.from_bytes(subject_bytes_prefixed[blockNumberOffset : blockNumberOffset + 2*31], "big")
        #rlpStartOffset = blockNumberOffset + FELT_SIZE * ids.BlockItemIdentifier.SIZE

        # padded leaf prefix + hi + lo + index
        #blockItemIdFeltsLen = ids.PADDING_SIZE + ids.BlockItemIdentifier.SIZE
        blockItemIdFeltsLen = ids.PADDING_SIZE + 1

        ids.query_offsets = ind = segments.add()
        for query_offset in flatten_list([range(qf['start'], qf['end']) for qf in claim['felt_ranges']]):
            memory[ind] = query_offset; ind += 1
        ids.query_offsets_len = min(ind - ids.query_offsets, ids.prefixed_subject_felts_len - blockItemIdFeltsLen - 1)
        memory[ind] = ids.query_offsets_len

        prev_block = [b for b in attestation_chain if int(b['block_number'])==(block_number - 1)]
        curr_block = [(ind, b) for (ind, b) in enumerate(attestation_chain) if int(b['block_number'])==block_number]
        
        ids.prev_block_digest = int(prev_block[0]['digest'])
        ids.curr_digest_from_attestation_chain = int(curr_block[0][1]['digest'])

        attestation_blocks_start_index = curr_block[0][0]
        ids.attestation_blocks_len = len(attestation_chain) - attestation_blocks_start_index

        ids.attestation_blocks = ind = segments.add()
        for attestation_block in attestation_chain[attestation_blocks_start_index:]:
            attestation_block_number = int(attestation_block['block_number'])

            memory[ind] = attestation_block_number; ind += 1
            memory[ind] = int(attestation_block['root']); ind += 1
            #memory[ind] = int(attestation_block['tx_root']); ind += 1
            #memory[ind] = int(attestation_block['rx_root']); ind += 1
            memory[ind] = int(attestation_block['prev_digest']); ind += 1
            memory[ind] = int(attestation_block['digest']); ind += 1
    %}

    assert digest_root_to_match_to_claim_root = proof_root;
    // hash (index || rlp felts)
    let hashed_claim_subject = pedersen_array(prefixed_subject_felts, array_len=prefixed_subject_felts_len);

    let (claim_index_from_path, zero_height_hamming_distance) = verify_merkle_path(
        root=proof_root,
        word_hash=claim_leaf_hash, 
        proof_items=proof_felts, 
        offsets=proof_offsets, 
        arity=proof_arity, 
        height=proof_height,
        i=0,
        ind=0,
        a=1,
        zero_height_hamming_distance=0,
        inner_node_prefix=inner_node_prefix
    );
    claim_index = prefixed_subject_felts[INDEX_FELT_OFFSET];
    // just for dev: ensure that claim_index_from_path is computed correctly
    let assert_claim_index_for_dev = (1 - out_of_bounds_flag) * (claim_index - claim_index_from_path);
    assert assert_claim_index_for_dev = 0;

    let rlp_felts = prefixed_subject_felts + PADDING_SIZE + INDEX_SIZE;
    // THE BELOW IS ONLY CORRECT FOR STANDALONE MERKLE TREE DESIGN, NOT MMR
    // the entire statement must equal to 0
    // statement breakdown:
    // in case prover set out_of_bounds_flag == 1:
    //  either claim_leaf_hash must be NULL_LEAF_HASH, or in case of full Merkle tree the convention is to send
    //  the last leaf and so, zero_height_hamming_distance == height
    let out_of_bounds_null_leaf_expr = NULL_LEAF_HASH - claim_leaf_hash;

    // assert hashed rlp-encoded (raw) subject equals to leaf felt unless it's a NULL leaf
    let claim_leaf_hash_assertion_expr = out_of_bounds_null_leaf_expr * (claim_leaf_hash - hashed_claim_subject);
    assert claim_leaf_hash_assertion_expr = 0;

//    let full_tree_height_assert_expr = (rlp_felts[0] - proof_height);
//    let out_of_bounds_full_tree_expr = (zero_height_hamming_distance - proof_height) * full_tree_height_assert_expr;
    let out_of_bounds_full_tree_expr = zero_height_hamming_distance - proof_height;
    
    let out_of_bounds_expr = out_of_bounds_flag * out_of_bounds_null_leaf_expr * out_of_bounds_full_tree_expr;

//    let in_bounds_expr = (1 - out_of_bounds_flag);
    
//    let expr_to_assert = out_of_bounds_expr + in_bounds_expr;
    let expr_to_assert = out_of_bounds_expr;

    assert expr_to_assert = 0;
    claim_index = prefixed_subject_felts[INDEX_FELT_OFFSET];
    // just for dev: ensure that claim_index_from_path is computed correctly
    let assert_claim_index_for_dev = (1 - out_of_bounds_flag) * (claim_index - claim_index_from_path);
    assert assert_claim_index_for_dev = 0;

    let rlp_felts = prefixed_subject_felts + PADDING_SIZE + INDEX_SIZE;
    // THE BELOW IS ONLY CORRECT FOR STANDALONE MERKLE TREE DESIGN, NOT MMR
    // the entire statement must equal to 0
    // statement breakdown:
    // in case prover set out_of_bounds_flag == 1:
    //  either claim_leaf_hash must be NULL_LEAF_HASH, or in case of full Merkle tree the convention is to send
    //  the last leaf and so, zero_height_hamming_distance == height
    let out_of_bounds_null_leaf_expr = NULL_LEAF_HASH - claim_leaf_hash;

    // assert hashed rlp-encoded (raw) subject equals to leaf felt unless it's a NULL leaf
    let claim_leaf_hash_assertion_expr = out_of_bounds_null_leaf_expr * (claim_leaf_hash - hashed_claim_subject);
    assert claim_leaf_hash_assertion_expr = 0;

//    let full_tree_height_assert_expr = (rlp_felts[0] - proof_height);
//    let out_of_bounds_full_tree_expr = (zero_height_hamming_distance - proof_height) * full_tree_height_assert_expr;
    let out_of_bounds_full_tree_expr = zero_height_hamming_distance - proof_height;
    
    let out_of_bounds_expr = out_of_bounds_flag * out_of_bounds_null_leaf_expr * out_of_bounds_full_tree_expr;

//    let in_bounds_expr = (1 - out_of_bounds_flag);
    
//    let expr_to_assert = out_of_bounds_expr + in_bounds_expr;
    let expr_to_assert = out_of_bounds_expr;

    assert expr_to_assert = 0;

    // compute digest using data from the claim and previous digest from the attestation chain
    //let d1 = pedersen_hash2(block_number, digest_tx_root);
    let d = pedersen_hash2(block_number, proof_root);
    //let d2 = pedersen_hash2(d1, digest_rx_root);
    let digest = pedersen_hash2(d, prev_block_digest);
 
    // assert equality of computed digest and the digest from the corresponding block in the attestation chain
    assert curr_digest_from_attestation_chain = digest;
 
    // assert continuity and return the last digest
    let continuity_attestation_checkpoint = generate_continuity_attestation(
        attestation_blocks=attestation_blocks, 
        len=attestation_blocks_len
    );
    // hash query offsets so it can be checked by claimer
    let query_hash = pedersen_array(query_offsets, array_len = query_offsets_len + 1);

    // output claim identifier fields
//    assert [output_ptr] = claim_index;
    assert [output_ptr] = claim_index_from_path;
    let output_ptr = output_ptr + 1;

    // output continuity attestation digest
    assert [output_ptr] = continuity_attestation_checkpoint.digest;
    let output_ptr = output_ptr + 1;

    // output continuity attestation checkpoint block number
    assert [output_ptr] = continuity_attestation_checkpoint.block_number;
    let output_ptr = output_ptr + 1;

    // output query offsets hash
    assert [output_ptr] = query_hash;
    let output_ptr = output_ptr + 1;

    // output rlp fields, if out_of_bounds_flag output nothing
//    output_array_at_offsets(rlp_felts, query_offsets, query_offsets_len * (1 - out_of_bounds_flag) + out_of_bounds_flag);
    // output rlp fields, if out_of_bounds_flag output nothing
//    output_array_at_offsets(rlp_felts, query_offsets, query_offsets_len * (1 - out_of_bounds_flag) + out_of_bounds_flag);
    output_array_at_offsets(rlp_felts, query_offsets, query_offsets_len);
    
    return ();
}
