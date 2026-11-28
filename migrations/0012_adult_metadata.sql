-- Online lookups of this library's series include entries a provider marks adult.
ALTER TABLE library ADD COLUMN adult_metadata BOOLEAN NOT NULL DEFAULT FALSE CHECK (adult_metadata IN (0, 1));
