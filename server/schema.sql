-- Owlingstone shared games (Cloudflare D1). One row per finished game, summary kept as JSON text.
CREATE TABLE IF NOT EXISTS games (
  id       INTEGER PRIMARY KEY,
  install  TEXT NOT NULL,          -- random id made by the app when sharing is turned on
  day      TEXT NOT NULL,          -- YYYY-MM-DD (UTC) the server received it
  build    TEXT NOT NULL,          -- app version
  mode     TEXT NOT NULL,          -- solo | duos
  hero     TEXT NOT NULL,
  place    INTEGER NOT NULL,
  comp     TEXT,
  quality  TEXT NOT NULL,          -- ok | afk | early_concede (see gameQuality in src/validate.js)
  summary  TEXT NOT NULL           -- validated JSON (see src/validate.js)
);
-- No index on install: only "Delete my shared games" looks games up by install (rare), and each index
-- would cost one more written row per game on the Free plan.
DROP INDEX IF EXISTS games_install_day;
CREATE INDEX IF NOT EXISTS games_day ON games (day);
-- Small counters so limits cost 2 row reads per request instead of counting today's games.
CREATE TABLE IF NOT EXISTS day_count (day TEXT PRIMARY KEY, n INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS install_count (install TEXT NOT NULL, day TEXT NOT NULL, n INTEGER NOT NULL,
  PRIMARY KEY (install, day));
-- Feedback written by players in the app (tray menu > Send feedback). Read only with the admin key.
CREATE TABLE IF NOT EXISTS feedback (
  id      INTEGER PRIMARY KEY,
  install TEXT NOT NULL,
  day     TEXT NOT NULL,
  build   TEXT NOT NULL,
  kind    TEXT NOT NULL,           -- idea | bug | other
  text    TEXT NOT NULL,           -- up to 1000 characters
  done    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS feedback_done ON feedback (done, id);
-- The per-install feedback limit counts today's rows for one install: indexed, so it reads a few rows.
CREATE INDEX IF NOT EXISTS feedback_install_day ON feedback (install, day);
