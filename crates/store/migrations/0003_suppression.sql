-- M2: recorded suppression of non-blocking validation warnings.
--
-- Blocking findings cannot be suppressed. A suppression is scoped to a draft,
-- a rule code, and the field it points at, and always carries a reason.

CREATE TABLE validation_suppression (
    id         TEXT PRIMARY KEY,
    draft_id   TEXT NOT NULL REFERENCES invoice_draft(id) ON DELETE CASCADE,
    rule_code  TEXT NOT NULL,
    field_path TEXT NOT NULL,
    reason     TEXT NOT NULL CHECK (length(trim(reason)) > 0),
    created_at TEXT NOT NULL,
    actor      TEXT NOT NULL,
    UNIQUE (draft_id, rule_code, field_path)
);

CREATE INDEX validation_suppression_draft_idx ON validation_suppression (draft_id);
