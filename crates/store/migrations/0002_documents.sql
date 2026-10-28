-- M2: documents, their lines, and artefacts.
--
-- One table holds both directions. An `outbound` document is one we issued: we
-- allocate its number from `sequence` and own its series. An `inbound` document
-- was received from a supplier: the number is theirs, verbatim, and `series` and
-- `sequence_value` stay null.
--
-- `owner_id` is always our own legal entity (the books this database holds).
-- `counterparty_id` is the buyer for outbound documents and the supplier for
-- inbound ones; both parties are also snapshotted in full, so a later edit or
-- archive of the party record cannot rewrite history.

ALTER TABLE invoice_draft_line ADD COLUMN vat_exemption_reason TEXT;
ALTER TABLE invoice_draft_line ADD COLUMN vat_exemption_reason_code TEXT;

-- A credit note draft records the invoice it corrects (BG-3).
ALTER TABLE invoice_draft ADD COLUMN preceding_invoice_id TEXT;
ALTER TABLE invoice_draft ADD COLUMN preceding_number TEXT;
ALTER TABLE invoice_draft ADD COLUMN preceding_issue_date TEXT;

CREATE TABLE blob (
    id          TEXT PRIMARY KEY,
    sha256      TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL,
    compression TEXT,
    payload     BLOB NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE document (
    id                      TEXT PRIMARY KEY,
    direction               TEXT NOT NULL CHECK (direction IN ('outbound', 'inbound')),
    owner_id                TEXT NOT NULL REFERENCES seller(id),
    counterparty_id         TEXT REFERENCES customer(id),
    doc_type                TEXT NOT NULL,
    series                  TEXT,
    number                  TEXT NOT NULL,
    sequence_value          INTEGER,
    currency                TEXT NOT NULL CHECK (length(currency) = 3),
    issue_date_local        TEXT NOT NULL,
    due_date_local          TEXT,
    delivery_date_local     TEXT,
    buyer_reference         TEXT,
    order_reference         TEXT,
    note                    TEXT,
    totals_json             TEXT NOT NULL,
    tax_breakdown_json      TEXT NOT NULL,
    issuer_snapshot_json    TEXT NOT NULL,
    recipient_snapshot_json TEXT NOT NULL,
    semantic_json           TEXT NOT NULL,
    semantic_hash           TEXT NOT NULL,
    preceding_invoice_id    TEXT,
    preceding_number        TEXT,
    preceding_issue_date    TEXT,
    state                   TEXT NOT NULL,
    recorded_at             TEXT NOT NULL,
    recorded_by_device      TEXT NOT NULL,
    created_at              TEXT NOT NULL
);

CREATE TABLE document_line (
    id              TEXT PRIMARY KEY,
    document_id     TEXT NOT NULL REFERENCES document(id),
    position        INTEGER NOT NULL,
    payload_json    TEXT NOT NULL,
    line_net_micros INTEGER NOT NULL,
    tax_micros      INTEGER NOT NULL,
    currency        TEXT NOT NULL,
    UNIQUE (document_id, position)
);

CREATE TABLE artefact (
    id               TEXT PRIMARY KEY,
    document_id      TEXT NOT NULL REFERENCES document(id),
    kind             TEXT NOT NULL,
    syntax_profile   TEXT,
    bytes_hash       TEXT NOT NULL,
    size_bytes       INTEGER NOT NULL,
    blob_id          TEXT NOT NULL REFERENCES blob(id),
    rendered_at      TEXT NOT NULL,
    renderer_version TEXT NOT NULL
);

-- One series for what we issue; the sender's own numbering for what we receive.
CREATE UNIQUE INDEX document_outbound_number
    ON document (owner_id, series, number) WHERE direction = 'outbound';
CREATE UNIQUE INDEX document_inbound_number
    ON document (owner_id, counterparty_id, number) WHERE direction = 'inbound';

CREATE INDEX document_owner_date ON document (owner_id, issue_date_local);
CREATE INDEX document_line_document ON document_line (document_id, position);
CREATE INDEX artefact_document ON artefact (document_id, kind);

-- Documents are write-once. These triggers are a backstop against future code
-- paths; status changes are append-only events, never updates to the document.
CREATE TRIGGER document_no_update
BEFORE UPDATE ON document
BEGIN
    SELECT RAISE(ABORT, 'documents are immutable; corrections are credit notes');
END;

CREATE TRIGGER document_no_delete
BEFORE DELETE ON document
BEGIN
    SELECT RAISE(ABORT, 'documents cannot be deleted');
END;

CREATE TRIGGER document_line_no_update
BEFORE UPDATE ON document_line
BEGIN
    SELECT RAISE(ABORT, 'document lines are immutable');
END;

CREATE TRIGGER document_line_no_delete
BEFORE DELETE ON document_line
BEGIN
    SELECT RAISE(ABORT, 'document lines cannot be deleted');
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
