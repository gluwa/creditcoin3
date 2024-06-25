// @generated automatically by Diesel CLI.

diesel::table! {
    signedattestation (id) {
        id -> Int4,
        chain_id -> Int8,
        header_number -> Int8,
        #[max_length = 64]
        header_hash -> Varchar,
        #[max_length = 64]
        tx_root -> Varchar,
        #[max_length = 64]
        rx_root -> Varchar,
        #[max_length = 64]
        digest -> Varchar,
        #[max_length = 64]
        prev_digest -> Nullable<Varchar>,
        #[max_length = 192]
        signature -> Varchar,
        attestors -> Array<Nullable<Text>>,
    }
}
