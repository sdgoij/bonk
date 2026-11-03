-- The payload audit asks, for every blob, whether any artefact points at it.
--
-- That lookup had no index: `artefact` was indexed by document and kind, not by
-- blob, so the orphan sweep scanned the whole artefact table once per blob.
-- On a 20 000-document archive that was five minutes of silence after the
-- payload pass, which reads as a hung process.
CREATE INDEX artefact_blob ON artefact (blob_id);
