-- Rolling back drops the recorded digest, not the artefact: the bytes and their
-- hashes are untouched, so a filed document still verifies and still exports.
-- What goes is the pointer to the layout that drew it, which is why this column
-- is not evidence the archive cannot afford to lose.
ALTER TABLE artefact DROP COLUMN template_digest;
