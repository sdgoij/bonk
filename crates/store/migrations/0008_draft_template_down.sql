-- The draft keeps its facts when this column goes; what is lost is only the
-- request for a particular layout, which the archive default then answers.
ALTER TABLE invoice_draft DROP COLUMN template_name;
