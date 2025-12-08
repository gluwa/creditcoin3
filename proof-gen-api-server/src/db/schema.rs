// @generated automatically by Diesel CLI.

diesel::table! {
    continuity_proofs (id) {
        id -> Int4,
        chain_key -> Int8,
        header_number -> Int8,
        continuity_proof -> Jsonb,
        created_at -> Nullable<Timestamp>,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    merkle_proofs (id) {
        id -> Int4,
        chain_key -> Int8,
        header_number -> Int8,
        tx_index -> Nullable<Int8>,
        #[max_length = 66]
        tx_hash -> Nullable<Varchar>,
        tx_bytes -> Nullable<Bytea>,
        merkle_proof -> Jsonb,
        #[max_length = 66]
        merkle_root -> Varchar,
        created_at -> Nullable<Timestamp>,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(continuity_proofs, merkle_proofs,);
