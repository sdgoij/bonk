-- Rollback of 0013.
--
-- Refused while the subledger holds anything: an open item is the detail behind a
-- control account that was reported on, and dropping it would make a filed figure
-- unexplainable.
CREATE TEMP TABLE open_item_rollback_guard (
    items INTEGER NOT NULL CONSTRAINT no_open_items CHECK (items = 0)
);
INSERT INTO open_item_rollback_guard (items) SELECT count(*) FROM open_item;
DROP TABLE open_item_rollback_guard;

DROP TRIGGER open_item_no_delete;
DROP TRIGGER open_item_no_update;
DROP TABLE open_item;
