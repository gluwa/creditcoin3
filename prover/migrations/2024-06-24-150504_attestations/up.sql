-- Create table for Attestation
CREATE TABLE Attestation (
    id SERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    merkle_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE,
    signature VARCHAR(192) NOT NULL,
    attestors TEXT [] NOT NULL
);

-- Create indexes on chain_id, header_number, and digest
CREATE INDEX attestation_idx_chain_id ON Attestation (chain_id);
CREATE INDEX attestation_idx_header_number ON Attestation (header_number);
CREATE INDEX attestation_idx_digest ON Attestation (digest);

-- Create table for source chain blocks included in fragments
CREATE TABLE BlockWithDigests (
    ID SERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    merkle_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE
);

CREATE INDEX block_with_digests_idx_chain_id ON BlockWithDigests (chain_id);
CREATE INDEX block_with_digests_idx_header_number ON BlockWithDigests (header_number);
CREATE INDEX block_with_digests_idx_digest ON BlockWithDigests (digest);

-- Create table for AttestationCheckpoints
CREATE TABLE AttestationCheckpoint (
    id SERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE
);

CREATE INDEX attestation_checkpoint_idx_chain_id ON AttestationCheckpoint (chain_id);
CREATE INDEX attestation_checkpoint_idx_block_number ON AttestationCheckpoint (block_number);
CREATE INDEX attestation_checkpoint_idx_digest ON AttestationCheckpoint (digest);
