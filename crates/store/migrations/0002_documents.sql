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

CREATE TABLE import_record (
    id               TEXT PRIMARY KEY,
    document_id      TEXT NOT NULL REFERENCES document(id),
    source_kind      TEXT NOT NULL,          -- ubl | cii | pdf_facturx | manual
    source_path      TEXT,
    payload_hash     TEXT NOT NULL,          -- SHA-256 of the bytes as received
    match_status     TEXT NOT NULL CHECK (match_status IN ('matched', 'unmatched')),
    match_confidence REAL,
    imported_at      TEXT NOT NULL
);

CREATE INDEX import_record_payload ON import_record (payload_hash);
CREATE INDEX import_record_document ON import_record (document_id);

-- A human decision about who a received document came from. The document keeps
-- the import-time match in `counterparty_id`; this table overrides it without
-- rewriting the document, and records who decided and when.
CREATE TABLE supplier_match (
    document_id TEXT PRIMARY KEY REFERENCES document(id),
    customer_id TEXT NOT NULL REFERENCES customer(id),
    decided_by  TEXT NOT NULL,
    decided_at  TEXT NOT NULL
);

-- Status transitions are append-only events. The document row never changes, so
-- "approved" or "paid" is a fact about what we decided, recorded when we
-- decided it, not a field someone can overwrite.
CREATE TABLE document_event (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    id          TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES document(id),
    kind        TEXT NOT NULL CHECK (kind IN ('approved', 'paid', 'archived')),
    at          TEXT NOT NULL,
    actor       TEXT NOT NULL,
    detail      TEXT
);

CREATE INDEX document_event_document ON document_event (document_id, seq);

CREATE TRIGGER document_event_no_update
BEFORE UPDATE ON document_event
BEGIN
    SELECT RAISE(ABORT, 'document events are append-only');
END;

CREATE TRIGGER document_event_no_delete
BEFORE DELETE ON document_event
BEGIN
    SELECT RAISE(ABORT, 'document events are append-only');
END;

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

-- Payloads are evidence too. Both writers (`record_received`, `attach_artefact`)
-- only insert, and every recorded hash points at these bytes, so a rewrite would
-- turn a verifiable archive into an unverifiable one.
CREATE TRIGGER blob_no_update
BEFORE UPDATE ON blob
BEGIN
    SELECT RAISE(ABORT, 'stored payloads are immutable');
END;

CREATE TRIGGER blob_no_delete
BEFORE DELETE ON blob
BEGIN
    SELECT RAISE(ABORT, 'stored payloads cannot be deleted');
END;

-- Search is a cache over `document`, kept in step by triggers, never the source
-- of truth. The columns are what someone types into a search box: the number
-- they are looking for, the party on the other side, a reference, and the note.
-- Dates, amounts, and status are structured filters over `document` and its
-- events, not FTS columns. `remove_diacritics 2` lets "cafe" find "Café".
CREATE VIRTUAL TABLE invoice_fts USING fts5(
    document_id UNINDEXED,
    number,
    party_name,
    reference,
    notes,
    tokenize = "unicode61 remove_diacritics 2"
);

-- A named set of find-panel filters. Convenience, not evidence: unlike every
-- other table here, this one is meant to be edited and deleted, so it has no
-- immutability triggers. It stores only what the panel can express today; a
-- column nothing writes is a column that lies.
CREATE TABLE saved_search (
    id         TEXT PRIMARY KEY,
    owner_id   TEXT NOT NULL REFERENCES seller(id),
    name       TEXT NOT NULL,
    query      TEXT NOT NULL,
    direction  TEXT CHECK (direction IN ('outbound', 'inbound')),
    status     TEXT CHECK (status IN ('issued', 'received', 'approved', 'paid', 'archived')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (owner_id, name)
);

-- `document` is write-once, so inserts are the only document-side change that
-- can reach the index. The party name prefers the party on file and falls back
-- to the snapshot taken when the document was recorded, so an unmatched inbound
-- document is still findable by the name its sender used.
CREATE TRIGGER document_search_insert AFTER INSERT ON document
BEGIN
    INSERT INTO invoice_fts (document_id, number, party_name, reference, notes)
    VALUES (
        NEW.id,
        NEW.number,
        COALESCE(
            (SELECT c.display_name FROM customer c WHERE c.id = NEW.counterparty_id),
            CASE NEW.direction
                WHEN 'inbound' THEN json_extract(NEW.issuer_snapshot_json, '$.legal_name')
                ELSE json_extract(NEW.recipient_snapshot_json, '$.legal_name')
            END,
            ''
        ),
        trim(COALESCE(NEW.buyer_reference, '') || ' ' || COALESCE(NEW.order_reference, '')),
        COALESCE(NEW.note, '')
    );
END;

-- A supplier decision changes the name the register shows, so it changes what the
-- index says. The upsert path may fire either trigger.
CREATE TRIGGER supplier_match_search_insert AFTER INSERT ON supplier_match
BEGIN
    UPDATE invoice_fts
       SET party_name = (SELECT c.display_name FROM customer c WHERE c.id = NEW.customer_id)
     WHERE document_id = NEW.document_id;
END;

CREATE TRIGGER supplier_match_search_update AFTER UPDATE ON supplier_match
BEGIN
    UPDATE invoice_fts
       SET party_name = (SELECT c.display_name FROM customer c WHERE c.id = NEW.customer_id)
     WHERE document_id = NEW.document_id;
END;
