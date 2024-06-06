%builtins output pedersen range_check bitwise

from starkware.cairo.common.alloc import alloc
from starkware.cairo.common.cairo_builtins import HashBuiltin
from starkware.cairo.common.hash import hash2
from starkware.cairo.common.math import assert_nn

//using FeltArray = (len: felt, data: felt*);

struct U256 {
    lo: felt,
    hi: felt,
}

struct AttestationBlock {
    block_number: U256,
    tx_root: felt,
    rx_root: felt,
    prev_digest: felt,
    digest: felt,
}

struct BlockItemIdentifier {
    block_number: U256,
    claim_index: felt,
}
struct ClaimIdentifier {
    claim_kind: felt,
    block_item_id: BlockItemIdentifier,
}

// struct VerifierPublicInput {
//     claim_proof_root: felt,
//     claim_id: ClaimIdentifier,
//     chain_id: felt,
//     claim_from: felt,
//     claim_to: felt,
//     continuity_checkpoint_digest: felt,
//     continuity_checkpoint_block_number: felt,
// }

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
    // ind,
    // a,
    inner_node_prefix
) {
//) -> felt {
    let h = hash_with_siblings(
                word_hash,
                proof_item=proof_items,
                offset=offsets[0],
                arity=arity,
                inner_node_prefix=inner_node_prefix
            );

    if (i == height - 1) {
        // check if the last hash and the root are identical
        assert root = h;
        return ();
//        return ind + a * offsets[0];
    }
    // calculate claim index from proof's offsets
    // let index = ind + a * offsets[0];
    // let a = a * arity;

    return verify_merkle_path(
                root,
                h,
                proof_items=proof_items+arity,
                offsets=offsets+1,
                arity=arity,
                height=height,
                i=i+1,
                // ind=index,
                // a=a,
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

func verify_continuity{pedersen_ptr: HashBuiltin*, range_check_ptr}(attestation_blocks: AttestationBlock*, len) -> AttestationBlock {
    assert_nn(len - 1);

    let d1 = pedersen_hash2(attestation_blocks[0].block_number.lo, attestation_blocks[0].block_number.hi);
    let d2 = pedersen_hash2(d1, attestation_blocks[0].tx_root);
    let d3 = pedersen_hash2(d2, attestation_blocks[0].rx_root);
    let digest = pedersen_hash2(d3, attestation_blocks[0].digest);

    if (len == 1) {
        return attestation_blocks[0];
    }

    return verify_continuity(attestation_blocks=attestation_blocks+AttestationBlock.SIZE, len=len-1);
}

// outputs an array of felts: the last felt output is the array's lenght
func output_array{output_ptr}(arr: felt*, len) {
    return output_array_aux(arr=arr, len=len, ind=0);
}

func output_array_aux{output_ptr}(arr: felt*, len, ind) {
    if (ind == len) {
        assert [output_ptr] = len;
        let output_ptr = output_ptr + 1;

        return ();
    }
    assert [output_ptr] = arr[ind];
    let output_ptr = output_ptr + 1;

    return output_array_aux(arr=arr, len=len, ind=ind+1);
}

func main{output_ptr, pedersen_ptr: HashBuiltin*, range_check_ptr, bitwise_ptr}() {
    alloc_locals;

    local proof_height;
    local proof_arity;
    local proof_root;
    local proof_felts: felt*;
    local proof_offsets: felt*;

    local prefixed_claim_rlp_felts: felt*;
    local prefixed_claim_rlp_felts_len;

    local dynamic_output_fields: felt*;
    local dynamic_output_fields_len: felt;

    local inner_node_prefix;
    local type_id;

    local claim_id: ClaimIdentifier;

    local digest_root_to_match_to_claim_root;
    local digest;
    local digest_tx_root;
    local digest_rx_root;
    local prev_block_digest;
    local curr_digest_from_attestation_chain;

    local attestation_blocks: AttestationBlock*;
    local attestation_blocks_len;
    // parse the proof and assign local variables
    %{
        from eth.vm.forks.cancun.transactions import CancunTransactionBuilder as TransactionBuilder
        from eth.vm.forks.cancun.receipts import CancunReceiptBuilder as ReceiptBuilder
        from rlp.sedes import CountableList
        #from rlp.logs import Log

        # parse bytes as 248-bit long elements (closest to canonical safe representation of felts)
        MAX_LEN_FELT = 31
        def flatten_list(nested_list):
            return [item for sublist in nested_list for item in sublist]
        # convert bytes to felts as big endian
        def bytes_to_felt_array(bytes):
            return [int.from_bytes(bytes[i:i+MAX_LEN_FELT], "big") for i in range(0, len(bytes), MAX_LEN_FELT)]

        claim = program_input['claim_with_merkle_proof']
        digest_roots = program_input['claim_digest_roots']
        attestation_chain = program_input['attestation_chain']['blocks']

        ids.claim_id.claim_kind = 1 if claim['claim_kind'] == "Tx" else 2

        #ids.claim_kind = 1 if claim['claim_kind'] == "Tx" else 2
        ids.digest_root_to_match_to_claim_root = int(digest_roots['tx_root'] if ids.claim_id.claim_kind == 1 else digest_roots['rx_root'])
        ids.digest_tx_root = int(digest_roots['tx_root'])
        ids.digest_rx_root = int(digest_roots['rx_root'])

        ids.proof_height = claim['height']
        ids.proof_arity = arity = claim['arity']

        path = claim['path']
        flat_path = [int(felt) for felt in flatten_list(path)]

        ids.proof_felts = path_felts_start = segments.add()
        ids.proof_offsets = path_offsets_start = segments.add()

        for i, felt_or_offset in enumerate(flat_path):
            if (i + 1) % (arity + 1) != 0:
                memory[path_felts_start] = felt_or_offset
                path_felts_start = path_felts_start + 1
            else:
                memory[path_offsets_start] = felt_or_offset
                path_offsets_start = path_offsets_start + 1

        #ids.claim = int(program_input['claim_felt'])
        ids.proof_root = int(claim['root'])

        claim_rlp_prefixed = [claim['leaf_hash_prefix']] + claim['claim_rlp']
        prefixed_claim_rlp_felts = bytes_to_felt_array(claim_rlp_prefixed)

        ids.prefixed_claim_rlp_felts = ind = segments.add()
        length = len(prefixed_claim_rlp_felts)
        for i in range(0, length):
            memory[ind] = prefixed_claim_rlp_felts[i]
            ind += 1
        # the last element contains the length of the claim without the prepended item
        memory[ind] = length
        # total length of the claim felt array plus the last item containing (length - 1)
        ids.prefixed_claim_rlp_felts_len = length + 1

        ids.inner_node_prefix = int(claim['inner_node_hash_prefix'])

        claimBytes = claim['claim_rlp']
        txRlpOffset = 0

        block_number = int.from_bytes(claimBytes[txRlpOffset : txRlpOffset + 32], "big")
        #block_number_felts = bytes_to_felt_array(claimBytes[txRlpOffset : txRlpOffset + 32])
        #ids.claim_id.block_item_id.block_number.lo = block_number_felts[0]
        #ids.claim_id.block_item_id.block_number.hi = block_number_felts[1]
        ids.claim_id.block_item_id.block_number.lo = int.from_bytes(claimBytes[txRlpOffset + 1 : txRlpOffset + 32], "big")
        ids.claim_id.block_item_id.block_number.hi = int.from_bytes(claimBytes[txRlpOffset : txRlpOffset + 1], "big")

        txRlpOffset += 32
        ids.claim_id.block_item_id.claim_index = int.from_bytes(claimBytes[txRlpOffset : txRlpOffset + 8], "big")

        txRlpOffset += 8

        ids.type_id = 0 if claimBytes[txRlpOffset] >= 0xc0 else claimBytes[txRlpOffset]
        #txRlpOffset += 1

        blockItemRlp = bytes(claimBytes[txRlpOffset:])

        txFieldSizesBytes = dict()
        txFieldSizesBytes["_chain_id"] = 8
        txFieldSizesBytes["_nonce"] = 32
        txFieldSizesBytes["_gas_price"] = 32
        txFieldSizesBytes["_gas"] = 32
        txFieldSizesBytes["_max_priority_fee_per_gas"] = 32
        txFieldSizesBytes["_max_fee_per_gas"] = 32
        txFieldSizesBytes["_to"] = 32
        txFieldSizesBytes["_value"] = 32
        txFieldSizesBytes["_max_fee_per_blob_gas"] = 32
        txFieldSizesBytes["_y_parity"] = 1
        txFieldSizesBytes["_v"] = 8
        txFieldSizesBytes["_r"] = 32
        txFieldSizesBytes["_s"] = 32

        rxFieldSizesBytes = dict()
        rxFieldSizesBytes["type_id"] = 1
        rxFieldSizesBytes["_gas_used"] = 32
        rxFieldSizesBytes["_bloom"] = 256
        rxFieldSizesBytes["_state_root"] = 32

        ids.dynamic_output_fields = ind = segments.add()
        if claim['claim_kind'] == "Tx":
            decodedTx = TransactionBuilder().decode(blockItemRlp)
            txDict = decodedTx._inner.__dict__ if hasattr(decodedTx, "_inner") else decodedTx.__dict__

            for k in txDict.keys():
                if k == "_cached_rlp":
                    continue

                val = txDict[k]
                if isinstance(val, int):
                    for felt in bytes_to_felt_array(val.to_bytes(txFieldSizesBytes[k], 'big')):
                        memory[ind] = felt; ind += 1
                elif isinstance(val, bytes):
                    for felt in bytes_to_felt_array(val):
                        memory[ind] = felt; ind += 1
                elif k == "_access_list":
                    for accountAccess in val:
                        for felt in bytes_to_felt_array(accountAccess["account"]):
                            memory[ind] = felt; ind += 1
                        for storageKey in accountAccess["storage_keys"]:
                            storageKeyBytes = storageKey.to_bytes(32, 'big')
                            storageKeyFeltLo = int.from_bytes(storageKeyBytes[1:32], 'big')
                            storageKeyFeltHi = int.from_bytes(storageKeyBytes[0:1], 'big')
                            memory[ind] = storageKeyFeltLo; ind += 1
                            memory[ind] = storageKeyFeltHi; ind += 1
                elif k == "_blob_versioned_hashes":
                    raise Exception("todo: convert blob_versioned_hashes into felts. Not done since no tx with blob_versioned_hashes was encountered so far.")
        else:
            decodedRx = ReceiptBuilder().decode(blockItemRlp)
            rxDict = decodedRx._inner.__dict__ if hasattr(decodedRx, "_inner") else decodedRx.__dict__

            for k in rxDict.keys():
                if k == "_cached_rlp":
                    continue

                val = rxDict[k]
                #if k == "_state_root":
                    #memory[ind] = val[0]; ind += 1
                    #continue
                if isinstance(val, int):
                    for felt in bytes_to_felt_array(val.to_bytes(rxFieldSizesBytes[k], 'big')):
                        memory[ind] = felt; ind += 1
                elif isinstance(val, bytes):
                    for felt in bytes_to_felt_array(val):
                        memory[ind] = felt; ind += 1
                elif k == "_logs":
                    for log in val:
                        memory[ind] = int.from_bytes(log["address"], 'big'); ind += 1
                        for topic in log["topics"]:
                            topicBytes = topic.to_bytes(32, 'big')
                            topicFeltLo, topicFeltHi = int.from_bytes(topicBytes[1:32], 'big'), int.from_bytes(topicBytes[0:1], 'big')
                            memory[ind] = topicFeltLo; ind += 1
                            memory[ind] = topicFeltHi; ind += 1
                        for felt in bytes_to_felt_array(log["data"]):
                            memory[ind] = felt; ind += 1

        ids.dynamic_output_fields_len = ind - ids.dynamic_output_fields

        prev_block = [b for b in attestation_chain if int(b['block_number'])==(block_number - 1)]
        curr_block = [(ind, b) for (ind, b) in enumerate(attestation_chain) if int(b['block_number'])==block_number]

        ids.prev_block_digest = int(prev_block[0]['digest'])
        ids.curr_digest_from_attestation_chain = int(curr_block[0][1]['digest'])

        attestation_blocks_start_index = curr_block[0][0]
        ids.attestation_blocks_len = len(attestation_chain) - attestation_blocks_start_index

        ids.attestation_blocks = ind = segments.add()
        for attestation_block in attestation_chain[attestation_blocks_start_index:]:
            attestation_block_number = int(attestation_block['block_number'])
            is_lo = (len(attestation_block['block_number']) < 32)

            block_number_lo = attestation_block_number if is_lo else int.from_bytes(attestation_block['block_number'][1 : 32], "big")
            block_number_hi = 0 if is_lo else int(attestation_block['block_number'][0])

            memory[ind] = block_number_lo
            ind += 1

            memory[ind] = block_number_hi
            ind += 1

            memory[ind] = int(attestation_block['tx_root'])
            ind += 1

            memory[ind] = int(attestation_block['rx_root'])
            ind += 1

            memory[ind] = int(attestation_block['prev_digest'])
            ind += 1

            memory[ind] = int(attestation_block['digest'])
            ind += 1
    %}

    assert digest_root_to_match_to_claim_root = proof_root;
    // claim hash is expected to be contained in the first proof item (merkle tree leaf) at it's offset
    let claim_hash = proof_felts[proof_offsets[0]];

    let hashed_claim_rlp = pedersen_array(prefixed_claim_rlp_felts, array_len=prefixed_claim_rlp_felts_len);
    // assert hashed rlp-encoded (raw) claim equals to claim felt
    assert hashed_claim_rlp = claim_hash;

    // let at = 1;
    // test_init_curr_item(proof_felts=proof_felts, at=at, proof_arity=proof_arity);

//    let claim_index_from_path = verify_merkle_path(
    verify_merkle_path(
        root=proof_root,
        word_hash=claim_hash,
        proof_items=proof_felts,
        offsets=proof_offsets,
        arity=proof_arity,
        height=proof_height,
        i=0,
        // ind=0,
        // a=1,
        inner_node_prefix=inner_node_prefix
    );

//    assert claim_index_from_path = claim_index;

    // compute digest using data from the claim and previous digest from the attestation chain
    let d1 = pedersen_hash2(claim_id.block_item_id.block_number.lo, claim_id.block_item_id.block_number.hi);
    let d2 = pedersen_hash2(d1, digest_tx_root);
    let d3 = pedersen_hash2(d2, digest_rx_root);
    let digest = pedersen_hash2(d3, prev_block_digest);

    // assert equality of computed digest and the digest from the corresponding block in the attestation chain
    assert curr_digest_from_attestation_chain = digest;

    // assert continuity and return the last digest
    let continuity_attestation_checkpoint = verify_continuity(attestation_blocks=attestation_blocks, len=attestation_blocks_len);

    // output claim identifier fields
    assert [cast(output_ptr, ClaimIdentifier*)] = claim_id;
    let output_ptr = output_ptr + ClaimIdentifier.SIZE;

    assert [output_ptr] = type_id;
    let output_ptr = output_ptr + 1;

    // output continuity attestation digest
    assert [output_ptr] = continuity_attestation_checkpoint.digest;
    let output_ptr = output_ptr + 1;

    // output continuity attestation checkpoint block number
    assert [cast(output_ptr, U256*)] = continuity_attestation_checkpoint.block_number;
    let output_ptr = output_ptr + U256.SIZE;

    //assert dynamic_output_fields[0] = 1;
    // output dynamic fields
    output_array(dynamic_output_fields, dynamic_output_fields_len);

    return ();
}
