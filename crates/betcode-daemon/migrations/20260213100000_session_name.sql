-- Add name column to sessions for user-visible labels.
ALTER TABLE sessions ADD COLUMN name TEXT NOT NULL DEFAULT '';
