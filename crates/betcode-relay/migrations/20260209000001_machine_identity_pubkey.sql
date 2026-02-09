-- Add identity_pubkey column to machines table for E2E encryption key exchange.
-- Stores the daemon's X25519 public key (32 bytes, hex-encoded).
ALTER TABLE machines ADD COLUMN identity_pubkey BLOB DEFAULT NULL;
