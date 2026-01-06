// @generated automatically by Diesel CLI.

diesel::table! {
    continuity_proofs (id) {
        id -> Int4,
        chain_key -> Int8,
        header_number -> Numeric,
        continuity_proof -> Jsonb,
        ends_in_attestation -> Bool,
        created_at -> Nullable<Timestamp>,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    continuity_blocks (id) {
        id -> Int4,
        chain_key -> Int8,
        header_number -> Numeric,
        digest -> Varchar,
        is_attestation -> Bool,
        is_checkpoint -> Bool,
    }
}
