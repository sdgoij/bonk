-- M10: settling an open item posted at a rate.
--
-- `amount_micros` stays the signed amount in the open item's document currency,
-- so the subledger balance is still the sum of the document's own money.
-- `transaction_amount_micros` is the same settlement in the payment's currency,
-- which caps a group and is what the queue shows as unapplied.
-- `base_amount_micros` is the carrying value released in the reporting currency,
-- and `fx_difference_micros` is the realised gain (positive is a gain). The
-- settlement entry's difference line is the sum of those gains.
--
-- Irreversible: dropping the payment-currency amount after a cross-currency match
-- would leave the queue's "left" figure and the settlement entry unexplainable,
-- and dropping the realised differences would erase a result already in the books.

ALTER TABLE allocation ADD COLUMN transaction_amount_micros INTEGER NOT NULL DEFAULT 0;

-- Before this migration every allocation was in its item's currency, so the
-- payment-currency amount is the same number.
UPDATE allocation SET transaction_amount_micros = amount_micros;

-- Where a realised difference lands. New archives get it from the starter chart;
-- this row gives an archive created before it the same account.
INSERT OR IGNORE INTO account (code, name, kind, rgs_code, parent_code, is_control, currency, archived, updated_at)
VALUES ('7900', 'Koersverschillen', 'expense', NULL, NULL, 0, NULL, 0,
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
