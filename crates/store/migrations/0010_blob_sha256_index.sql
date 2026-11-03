-- Imports are matched to their payload by hash, and the audit asks, per import,
-- whether that payload is still on file. Without this index the lookup scanned
-- the whole blob table — payload column and all — once per import.
CREATE INDEX blob_sha256 ON blob (sha256);
