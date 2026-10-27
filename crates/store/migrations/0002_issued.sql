-- M1: issued documents, artefacts, and the ledger's future anchors.

ALTER TABLE invoice_draft_line ADD COLUMN vat_exemption_reason TEXT;
ALTER TABLE invoice_draft_line ADD COLUMN vat_exemption_reason_code TEXT;

CREATE TABLE blob (
    id          TEXT PRIMARY KEY,
    sha256      TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL,
    compression TEXT,
    payload     BLOB NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE issued_invoice (
    id                   TEXT PRIMARY KEY,
    seller_id            TEXT NOT NULL REFERENCES seller(id),
    doc_type             TEXT NOT NULL,
    series               TEXT NOT NULL,
    number               TEXT NOT NULL,
    sequence_value       INTEGER NOT NULL,
    currency             TEXT NOT NULL CHECK (length(currency) = 3),
    issue_date_local     TEXT NOT NULL,
    due_date_local       TEXT,
    delivery_date_local  TEXT,
    totals_json          TEXT NOT NULL,
    tax_breakdown_json   TEXT NOT NULL,
    seller_snapshot_json TEXT NOT NULL,
    buyer_snapshot_json  TEXT NOT NULL,
    semantic_json        TEXT NOT NULL,
    semantic_hash        TEXT NOT NULL,
    rounded_at           TEXT NOT NULL,
    posting_profile_id   TEXT,
    state                TEXT NOT NULL,
    issued_at            TEXT NOT NULL,
    issued_by_device     TEXT NOT NULL,
    created_at           TEXT NOT NULL,
    UNIQUE (seller_id, series, number)
);

CREATE TABLE issued_line (
    id               TEXT PRIMARY KEY,
    invoice_id       TEXT NOT NULL REFERENCES issued_invoice(id),
    position         INTEGER NOT NULL,
    payload_json     TEXT NOT NULL,
    line_net_micros  INTEGER NOT NULL,
    tax_micros       INTEGER NOT NULL,
    currency         TEXT NOT NULL,
    UNIQUE (invoice_id, position)
);

CREATE TABLE artefact (
    id                 TEXT PRIMARY KEY,
    invoice_id         TEXT REFERENCES issued_invoice(id),
    imported_invoice_id TEXT,
    kind               TEXT NOT NULL,
    syntax_profile     TEXT,
    bytes_hash         TEXT NOT NULL,
    size_bytes         INTEGER NOT NULL,
    blob_id            TEXT NOT NULL REFERENCES blob(id),
    rendered_at        TEXT NOT NULL,
    renderer_version   TEXT NOT NULL
);

CREATE INDEX issued_invoice_seller_idx ON issued_invoice (seller_id, issue_date_local);
CREATE INDEX issued_line_invoice_idx ON issued_line (invoice_id, position);
CREATE INDEX artefact_invoice_idx ON artefact (invoice_id, kind);

-- Issued documents are write-once. These triggers are a backstop against future
-- code paths; the repositories never issue UPDATE or DELETE on them.
CREATE TRIGGER issued_invoice_no_update
BEFORE UPDATE ON issued_invoice
BEGIN
    SELECT RAISE(ABORT, 'issued invoices are immutable; issue a credit note');
END;

CREATE TRIGGER issued_invoice_no_delete
BEFORE DELETE ON issued_invoice
BEGIN
    SELECT RAISE(ABORT, 'issued invoices cannot be deleted');
END;

CREATE TRIGGER issued_line_no_update
BEFORE UPDATE ON issued_line
BEGIN
    SELECT RAISE(ABORT, 'issued lines are immutable');
END;

CREATE TRIGGER issued_line_no_delete
BEFORE DELETE ON issued_line
BEGIN
    SELECT RAISE(ABORT, 'issued lines cannot be deleted');
END;

CREATE TRIGGER artefact_no_update
BEFORE UPDATE ON artefact
BEGIN
    SELECT RAISE(ABORT, 'artefacts are immutable');
END;

CREATE TRIGGER artefact_no_delete
BEFORE DELETE ON artefact
BEGIN
    SELECT RAISE(ABORT, 'artefacts cannot be deleted');
END;
