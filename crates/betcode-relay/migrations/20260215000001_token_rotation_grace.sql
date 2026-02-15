-- Soft-rotation columns for refresh token grace window.
-- When a token is rotated, rotated_at records when it happened.
-- successor_id links to the new token that replaced it.
-- A recently-rotated token (within the grace window) can still be
-- presented to obtain a new token pair, handling the case where
-- the client never received the rotation response.
ALTER TABLE tokens ADD COLUMN rotated_at INTEGER;
ALTER TABLE tokens ADD COLUMN successor_id TEXT REFERENCES tokens(id);
