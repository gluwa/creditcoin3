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
CREATE TABLE BlockWithDigest (
    ID SERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    merkle_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE
);

CREATE INDEX block_with_digest_idx_chain_id ON BlockWithDigest (chain_id);
CREATE INDEX block_with_digest_idx_header_number ON BlockWithDigest (header_number);
CREATE INDEX block_with_digest_idx_digest ON BlockWithDigest (digest);

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

-- Create table for last checkpoint we know the local cache is fully caught up to
CREATE TABLE FullyCachedThrough (
   onerow_id BOOL PRIMARY KEY DEFAULT true, 
   digest VARCHAR(64) NOT NULL UNIQUE,
   CONSTRAINT onerow_uni CHECK (onerow_id)
);
