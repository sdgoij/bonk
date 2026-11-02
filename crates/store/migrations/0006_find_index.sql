-- The find index answers the list on its own.
--
-- 0002 kept only the words in `invoice_fts`, so every search joined `document`
-- by primary key to reach the owner, the date, and the status it filters and
-- orders by. That join reads a whole document row — several kilobytes of JSON —
-- for every match: 19 000 matches took 48 ms, of which the text match itself was
-- 0.2 ms. Reading the index alone and fetching just the page is 51 rows.
--
-- So the index carries the columns the find panel filters and orders by, and the
-- words stay for matching. It is still a cache over `document`: every column
-- here is derived, the triggers below keep it in step, and the backfill below is
-- the proof it can be rebuilt from the tables it describes.
--
-- `owner_id` is indexed rather than merely stored, because scoping a search to
-- its seller is part of every query. Comparing a stored column after matching
-- materialises a row per match; intersecting doclists in the index does not.
-- It is stored without its hyphens so the identifier is one token, which is what
-- lets a query name it: a phrase of five hyphen-separated tokens costs 56 ms
-- where a single token costs nothing measurable.
--
-- FTS5 cannot add columns to a virtual table, so the table is rebuilt. Nothing
-- references it except the triggers, which are dropped here and recreated.
DROP TRIGGER document_search_insert;
DROP TRIGGER supplier_match_search_insert;
DROP TRIGGER supplier_match_search_update;
DROP TABLE invoice_fts;

CREATE VIRTUAL TABLE invoice_fts USING fts5(
    document_id UNINDEXED,
    owner_id,
    direction UNINDEXED,
    issue_date_local UNINDEXED,
    status UNINDEXED,
    number,
    party_name,
    reference,
    notes,
    tokenize = "unicode61 remove_diacritics 2"
);

-- The same derivations the triggers use, applied to every document already on
-- file: the party a human decided on, then the party on file, then the snapshot;
-- the status from the newest event, or the state the document arrived in.
INSERT INTO invoice_fts
    (document_id, owner_id, direction, issue_date_local, status, number, party_name, reference, notes)
SELECT d.id,
       replace(d.owner_id, '-', ''),
       d.direction,
       d.issue_date_local,
       COALESCE(
           (SELECT e.kind FROM document_event e WHERE e.document_id = d.id ORDER BY e.seq DESC LIMIT 1),
           d.state
       ),
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

-- `document` is write-once, so an insert is the only document-side change that
-- can reach the index. The party name prefers the party on file and falls back
-- to the snapshot taken when the document was recorded, so an unmatched inbound
-- document is still findable by the name its sender used.
CREATE TRIGGER document_search_insert AFTER INSERT ON document
BEGIN
    INSERT INTO invoice_fts
        (document_id, owner_id, direction, issue_date_local, status, number, party_name, reference, notes)
    VALUES (
        NEW.id,
        replace(NEW.owner_id, '-', ''),
        NEW.direction,
        NEW.issue_date_local,
        COALESCE(
            (SELECT e.kind FROM document_event e WHERE e.document_id = NEW.id ORDER BY e.seq DESC LIMIT 1),
            NEW.state
        ),
        NEW.number,
        COALESCE(
            (SELECT c.display_name FROM supplier_match m JOIN customer c ON c.id = m.customer_id
              WHERE m.document_id = NEW.id),
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

-- Status is an append-only event log, so the newest insert is the current state
-- and the index needs no re-derivation.
CREATE TRIGGER document_event_search_status AFTER INSERT ON document_event
BEGIN
    UPDATE invoice_fts SET status = NEW.kind WHERE document_id = NEW.document_id;
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

-- Renaming a party has to reach the index too, or the list shows the new name
-- while the words that find the document are the old one. A document a human has
-- matched to a supplier keeps that decision's name instead of the party's.
CREATE TRIGGER customer_search_rename AFTER UPDATE OF display_name ON customer
BEGIN
    UPDATE invoice_fts
       SET party_name = NEW.display_name
     WHERE document_id IN (
               SELECT d.id FROM document d
                WHERE d.counterparty_id = NEW.id
                  AND NOT EXISTS (SELECT 1 FROM supplier_match m WHERE m.document_id = d.id)
           )
        OR document_id IN (
               SELECT m.document_id FROM supplier_match m WHERE m.customer_id = NEW.id
           );
END;
