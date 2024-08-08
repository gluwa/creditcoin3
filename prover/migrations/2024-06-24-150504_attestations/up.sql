-- Create table for AttestationCheckpoint
CREATE TABLE AttestationCheckpoint (
    id SERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    tx_root VARCHAR(64) NOT NULL,
    rx_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE,
    signature VARCHAR(192) NOT NULL,
    attestors TEXT [] NOT NULL
);

-- Create indexes on chain_id, header_number, and digest
CREATE INDEX signed_attestation_idx_chain_id ON AttestationCheckpoint (chain_id);
CREATE INDEX signed_attestation_idx_header_number ON AttestationCheckpoint (header_number);
CREATE INDEX signed_attestation_idx_digest ON AttestationCheckpoint (digest);

CREATE TABLE Attestation (
    ID SERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    tx_root VARCHAR(64) NOT NULL,
    rx_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE
);

CREATE INDEX attestation_idx_chain_id ON Attestation (chain_id);
CREATE INDEX attestation_idx_header_number ON Attestation (header_number);
CREATE INDEX attestation_idx_digest ON Attestation (digest);
