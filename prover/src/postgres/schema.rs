// @generated automatically by Diesel CLI.

diesel::table! {
    attestation (id) {
        id -> Int4,
        chain_id -> Int8,
        header_number -> Int8,
        #[max_length = 64]
        header_hash -> Varchar,
        #[max_length = 64]
        merkle_root -> Varchar,
        #[max_length = 64]
        digest -> Varchar,
        #[max_length = 64]
        prev_digest -> Nullable<Varchar>,
        #[max_length = 192]
        signature -> Varchar,
        attestors -> Array<Nullable<Text>>,
    }
}

diesel::table! {
    attestationcheckpoint (id) {
        id -> Int4,
        chain_id -> Int8,
        block_number -> Int8,
        #[max_length = 64]
        digest -> Varchar,
        #[max_length = 64]
        prev_digest -> Nullable<Varchar>,
    }
}

diesel::table! {
    blockwithdigest (id) {
        id -> Int4,
        chain_id -> Int8,
        header_number -> Int8,
        #[max_length = 64]
        header_hash -> Varchar,
        #[max_length = 64]
        merkle_root -> Varchar,
        #[max_length = 64]
        digest -> Varchar,
        #[max_length = 64]
        prev_digest -> Nullable<Varchar>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(attestation, attestationcheckpoint, blockwithdigest,);
