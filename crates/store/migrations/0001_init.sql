-- M0 schema: the minimum that proves encrypted storage, migrations, and the
-- shape of the core records. Later milestones add issued documents, artefacts,
-- outbox, sync, and the accounting layer.

CREATE TABLE customer (
    id                 TEXT PRIMARY KEY,
    display_name       TEXT NOT NULL,
    legal_name         TEXT,
    vat_id             TEXT,
    kvk_number         TEXT,
    peppol_id          TEXT,
    address_json       TEXT NOT NULL DEFAULT '{}',
    contact_json       TEXT,
    buyer_reference    TEXT,
    payment_terms_days INTEGER,
    archived           INTEGER NOT NULL DEFAULT 0,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    revision           INTEGER NOT NULL DEFAULT 0,
    hlc                TEXT NOT NULL DEFAULT ''
);

CREATE TABLE seller (
    id                 TEXT PRIMARY KEY,
    legal_name         TEXT NOT NULL,
    trading_name       TEXT,
    vat_id             TEXT,
    kvk_number         TEXT,
    peppol_id          TEXT,
    eas_code           TEXT,
    address_json       TEXT NOT NULL DEFAULT '{}',
    contact_json       TEXT,
    bank_json          TEXT,
    default_currency   TEXT NOT NULL CHECK (length(default_currency) = 3),
    default_locale     TEXT NOT NULL DEFAULT 'nl-NL',
    logo_blob_id       TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    revision           INTEGER NOT NULL DEFAULT 1,
    hlc                TEXT NOT NULL DEFAULT ''
);

CREATE TABLE tax_code (
    id                    TEXT PRIMARY KEY,
    seller_id             TEXT NOT NULL REFERENCES seller(id),
    jurisdiction          TEXT NOT NULL DEFAULT 'NL',
    label                 TEXT NOT NULL,
    vat_category          TEXT NOT NULL,
    vat_rate_bp           INTEGER NOT NULL CHECK (vat_rate_bp BETWEEN 0 AND 10000),
    reverse_charge        INTEGER NOT NULL DEFAULT 0,
    exemption_reason      TEXT,
    exemption_reason_code TEXT,
    archived              INTEGER NOT NULL DEFAULT 0,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL
);

CREATE TABLE app_setting (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    scope      TEXT NOT NULL DEFAULT 'app'
               CHECK (scope IN ('app', 'seller', 'device_local')),
    updated_at TEXT NOT NULL
);

CREATE TABLE sequence (
    owner_id   TEXT NOT NULL REFERENCES seller(id),
    doc_type   TEXT NOT NULL,
    series     TEXT NOT NULL,
    year       INTEGER NOT NULL,
    next_value INTEGER NOT NULL,
    lease_end  INTEGER,
    PRIMARY KEY (owner_id, doc_type, series, year)
);

CREATE TABLE invoice_draft (
    id                  TEXT PRIMARY KEY,
    owner_id            TEXT NOT NULL REFERENCES seller(id),
    counterparty_id     TEXT REFERENCES customer(id),
    doc_type            TEXT NOT NULL DEFAULT '380',
    currency            TEXT NOT NULL CHECK (length(currency) = 3),
    issue_date_local    TEXT NOT NULL,
    due_date_local      TEXT,
    delivery_date_local TEXT,
    buyer_reference     TEXT,
    order_reference     TEXT,
    project_reference   TEXT,
    notes_json          TEXT,
    payment_terms_days  INTEGER,
    raw_input_json      TEXT,
    revision            INTEGER NOT NULL DEFAULT 0,
    hlc                 TEXT NOT NULL DEFAULT '',
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE invoice_draft_line (
    id                    TEXT PRIMARY KEY,
    draft_id              TEXT NOT NULL REFERENCES invoice_draft(id) ON DELETE CASCADE,
    position              INTEGER NOT NULL,
    name                  TEXT NOT NULL,
    description           TEXT,
    quantity_micros       INTEGER NOT NULL,
    unit_code             TEXT NOT NULL DEFAULT 'C62',
    unit_price_micros     INTEGER NOT NULL,
    price_base_qty_micros INTEGER NOT NULL DEFAULT 1000000,
    vat_category          TEXT NOT NULL,
    vat_rate_bp           INTEGER NOT NULL,
    tax_code_id           TEXT REFERENCES tax_code(id),
    item_ref              TEXT,
    deleted               INTEGER NOT NULL DEFAULT 0,
    revision              INTEGER NOT NULL DEFAULT 0,
    hlc                   TEXT NOT NULL DEFAULT ''
);

CREATE TABLE audit_event (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT NOT NULL UNIQUE,
    at           TEXT NOT NULL,
    actor        TEXT NOT NULL,
    kind         TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    prev_hash    TEXT NOT NULL,
    hash         TEXT NOT NULL
);

CREATE INDEX customer_display_name_idx ON customer (display_name);
CREATE INDEX invoice_draft_owner_idx ON invoice_draft (owner_id, issue_date_local);
CREATE INDEX invoice_draft_line_draft_idx ON invoice_draft_line (draft_id, position);
CREATE INDEX tax_code_seller_idx ON tax_code (seller_id, archived);
CREATE INDEX audit_event_subject_idx ON audit_event (subject_kind, subject_id, seq);
