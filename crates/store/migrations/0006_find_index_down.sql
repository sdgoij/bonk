-- Back to the words-only index of 0002.
--
-- Rolling back loses no evidence: every row is derived from `document`, its
-- events, and the parties, so the table is rebuilt from those same sources. The
-- list reads through the join again, which is correct and slower.
DROP TRIGGER document_search_insert;
DROP TRIGGER document_event_search_status;
DROP TRIGGER supplier_match_search_insert;
DROP TRIGGER supplier_match_search_update;
DROP TRIGGER customer_search_rename;
DROP TABLE invoice_fts;

CREATE VIRTUAL TABLE invoice_fts USING fts5(
    document_id UNINDEXED,
    number,
    party_name,
    reference,
    notes,
    tokenize = "unicode61 remove_diacritics 2"
);

INSERT INTO invoice_fts (document_id, number, party_name, reference, notes)
SELECT d.id,
       d.number,
       COALESCE(
           (SELECT c.display_name FROM supplier_match m JOIN customer c ON c.id = m.customer_id
             WHERE m.document_id = d.id),
           (SELECT c.display_name FROM customer c WHERE c.id = d.counterparty_id),
           CASE d.direction WHEN 'inbound'
               THEN json_extract(d.issuer_snapshot_json, '$.legal_name')
               ELSE json_extract(d.recipient_snapshot_json, '$.legal_name')
           END,
           ''
       ),
       trim(COALESCE(d.buyer_reference, '') || ' ' || COALESCE(d.order_reference, '')),
       COALESCE(d.note, '')
FROM document d;

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
