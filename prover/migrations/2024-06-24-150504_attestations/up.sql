-- Create table for Attestation
CREATE TABLE Attestation (
    id SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    merkle_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE,
    signature VARCHAR(192) NOT NULL,
    attestors TEXT [] NOT NULL
);

-- Create indexes on chain_key, header_number, and digest
CREATE INDEX attestation_idx_chain_key ON Attestation (chain_key);
CREATE INDEX attestation_idx_header_number ON Attestation (header_number);
CREATE INDEX attestation_idx_digest ON Attestation (digest);
CREATE UNIQUE INDEX attestation_idx_digest_and_prev on Attestation (digest, prev_digest);

-- Create table for source chain blocks included in fragments
CREATE TABLE BlockWithDigest (
    ID SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    merkle_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE
);

CREATE INDEX block_with_digest_idx_chain_key ON BlockWithDigest (chain_key);
CREATE INDEX block_with_digest_idx_header_number ON BlockWithDigest (header_number);
CREATE INDEX block_with_digest_idx_digest ON BlockWithDigest (digest);

-- Create table for AttestationCheckpoints
CREATE TABLE AttestationCheckpoint (
    id SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE
);

CREATE INDEX attestation_checkpoint_idx_chain_key ON AttestationCheckpoint (chain_key);
CREATE INDEX attestation_checkpoint_idx_block_number ON AttestationCheckpoint (block_number);
CREATE INDEX attestation_checkpoint_idx_digest ON AttestationCheckpoint (digest);

-- Create table storing the checkpoint we've successfully cached up to.
-- All history before this checkpoint is locally stored.
CREATE TABLE CachedUpTo (
   chain_key BIGINT PRIMARY KEY, 
   digest VARCHAR(64) NOT NULL UNIQUE
);
