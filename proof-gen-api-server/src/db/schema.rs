// @generated automatically by Diesel CLI.

diesel::table! {
    continuity_proofs (id) {
        id -> Int4,
        chain_key -> Int8,
        header_number -> Int8,
        continuity_proof -> Jsonb,
        ends_in_attestation -> Bool,
        created_at -> Nullable<Timestamp>,
        updated_at -> Nullable<Timestamp>,
    }
}
