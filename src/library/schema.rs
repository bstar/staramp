//! Index schema.
//!
//! Two decisions here are worth defending, because both are ways music players
//! commonly lose user data.
//!
//! **`track_stat` is keyed by URI, not by `track.id`.** Re-indexing deletes and
//! recreates `track` rows. Stats hung off a row id are silently wiped by a
//! rescan; keyed by the stable URI they survive rescans, remounts, and a track
//! temporarily disappearing because a USB disk was unplugged. It costs one join.
//!
//! **`playlist_item` stores `uri` and `raw_line` unconditionally.** 243 entries
//! across the reference library's playlists do not resolve today. Modelled as
//! `item -> track_id` they would evaporate the first time a playlist is written
//! back, permanently damaging files curated since 2022.

pub const SCHEMA_VERSION: i32 = 1;

pub const PRAGMAS: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size   = -65536;
";

pub const SCHEMA: &str = r#"
-- ---------- filesystem layer ----------
CREATE TABLE IF NOT EXISTS dir (
  id          INTEGER PRIMARY KEY,
  rel_path    TEXT NOT NULL UNIQUE,
  mtime_ns    INTEGER,
  art_file_id INTEGER REFERENCES file(id) ON DELETE SET NULL,
  art_state   INTEGER NOT NULL DEFAULT 0,   -- 0 unknown, 1 resolved, 2 none
  scan_gen    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS file (
  id       INTEGER PRIMARY KEY,
  dir_id   INTEGER NOT NULL REFERENCES dir(id) ON DELETE CASCADE,
  rel_path TEXT NOT NULL UNIQUE,
  size     INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  kind     INTEGER NOT NULL,                -- 0 audio, 1 cue, 2 image, 3 playlist
  scan_gen INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS file_dir_idx ON file(dir_id, kind);
CREATE INDEX IF NOT EXISTS file_gen_idx ON file(scan_gen);

-- ---------- logical layer ----------
CREATE TABLE IF NOT EXISTS album (
  id            INTEGER PRIMARY KEY,
  name          TEXT,
  album_artist  TEXT,
  year          INTEGER,
  art_file_id   INTEGER REFERENCES file(id) ON DELETE SET NULL,
  rg_album_gain REAL,
  rg_album_peak REAL,
  UNIQUE(name, album_artist, year)
);

CREATE TABLE IF NOT EXISTS track (
  id  INTEGER PRIMARY KEY,
  uri TEXT NOT NULL UNIQUE,          -- 'A/B/01.flac' | 'A/B/x.cue/track0001'

  file_id        INTEGER NOT NULL REFERENCES file(id) ON DELETE CASCADE,
  cue_file_id    INTEGER REFERENCES file(id) ON DELETE CASCADE,
  cue_ordinal    INTEGER,            -- the NNNN in trackNNNN, positional
  cue_track_no   INTEGER,            -- as written in the sheet
  cue_file_index INTEGER,            -- which FILE stanza (multi-FILE sheets)
  start_frame    INTEGER NOT NULL DEFAULT 0,
  end_frame      INTEGER,            -- NULL = to EOF
  -- A disc-image cue hides its backing file; a per-track cue must not.
  hidden         INTEGER NOT NULL DEFAULT 0,

  title        TEXT,
  artist       TEXT,
  album_artist TEXT,
  album        TEXT,
  album_id     INTEGER REFERENCES album(id) ON DELETE SET NULL,
  composer     TEXT,
  genre        TEXT,
  track_no     INTEGER,
  track_total  INTEGER,
  disc_no      INTEGER,
  disc_total   INTEGER,
  year         INTEGER,
  date         TEXT,

  codec        TEXT NOT NULL,
  duration_ms  INTEGER,
  sample_rate  INTEGER,
  bit_depth    INTEGER,
  channels     INTEGER,
  bitrate_kbps INTEGER,
  is_lossless  INTEGER NOT NULL DEFAULT 0,
  file_size    INTEGER,

  rg_track_gain REAL,
  rg_track_peak REAL,
  rg_album_gain REAL,
  rg_album_peak REAL,
  rg_source     INTEGER NOT NULL DEFAULT 0,  -- 0 none, 1 tags, 2 scanned

  mb_recording_id TEXT,
  mb_release_id   TEXT,
  mb_artist_id    TEXT,

  added_at    INTEGER NOT NULL,
  modified_at INTEGER NOT NULL,
  scan_gen    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS track_artist_idx  ON track(artist);
CREATE INDEX IF NOT EXISTS track_aartist_idx ON track(album_artist, album, disc_no, track_no);
CREATE INDEX IF NOT EXISTS track_album_idx   ON track(album_id, disc_no, track_no);
CREATE INDEX IF NOT EXISTS track_genre_idx   ON track(genre);
CREATE INDEX IF NOT EXISTS track_year_idx    ON track(year);
CREATE INDEX IF NOT EXISTS track_codec_idx   ON track(codec, sample_rate, bit_depth);
CREATE INDEX IF NOT EXISTS track_added_idx   ON track(added_at DESC);
CREATE INDEX IF NOT EXISTS track_dur_idx     ON track(duration_ms);
CREATE INDEX IF NOT EXISTS track_file_idx    ON track(file_id);
CREATE INDEX IF NOT EXISTS track_cue_idx     ON track(cue_file_id, cue_ordinal);
CREATE INDEX IF NOT EXISTS track_gen_idx     ON track(scan_gen);

-- ---------- stats: keyed by URI so a rescan cannot wipe them ----------
CREATE TABLE IF NOT EXISTS track_stat (
  uri              TEXT PRIMARY KEY,
  play_count       INTEGER NOT NULL DEFAULT 0,
  skip_count       INTEGER NOT NULL DEFAULT 0,
  last_played_at   INTEGER,
  first_played_at  INTEGER,
  total_played_ms  INTEGER NOT NULL DEFAULT 0,
  completion_ratio REAL,
  rating           INTEGER,
  loved            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS stat_play_idx  ON track_stat(play_count DESC);
CREATE INDEX IF NOT EXISTS stat_last_idx  ON track_stat(last_played_at DESC);
CREATE INDEX IF NOT EXISTS stat_loved_idx ON track_stat(loved) WHERE loved = 1;

CREATE TABLE IF NOT EXISTS play_history (
  id        INTEGER PRIMARY KEY,
  uri       TEXT NOT NULL,
  played_at INTEGER NOT NULL,
  played_ms INTEGER,
  completed INTEGER
);
CREATE INDEX IF NOT EXISTS history_at_idx ON play_history(played_at DESC);

-- ---------- playlists ----------
CREATE TABLE IF NOT EXISTS playlist (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  source_path TEXT,
  kind        INTEGER NOT NULL DEFAULT 0,   -- 0 static, 1 smart
  created_at  INTEGER,
  updated_at  INTEGER
);

CREATE TABLE IF NOT EXISTS playlist_item (
  playlist_id INTEGER NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
  pos         INTEGER NOT NULL,
  uri         TEXT NOT NULL,                -- always stored, even if unresolvable
  track_id    INTEGER REFERENCES track(id) ON DELETE SET NULL,
  raw_line    TEXT,                         -- verbatim, for lossless write-back
  PRIMARY KEY (playlist_id, pos)
);

CREATE TABLE IF NOT EXISTS smart_playlist (
  id      INTEGER PRIMARY KEY REFERENCES playlist(id) ON DELETE CASCADE,
  expr    TEXT NOT NULL,
  sort    TEXT,
  limit_n INTEGER
);

-- ---------- search ----------
CREATE VIRTUAL TABLE IF NOT EXISTS track_fts USING fts5(
  title, artist, album, album_artist, genre,
  content='track', content_rowid='id',
  tokenize="unicode61 remove_diacritics 2"
);

-- ---------- scan bookkeeping ----------
CREATE TABLE IF NOT EXISTS scan_state (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  generation  INTEGER NOT NULL,
  phase       INTEGER NOT NULL,
  started_at  INTEGER,
  files_total INTEGER,
  files_done  INTEGER
);

CREATE TABLE IF NOT EXISTS rg_queue (
  uri      TEXT PRIMARY KEY,
  album_id INTEGER,
  priority INTEGER NOT NULL DEFAULT 0,
  attempts INTEGER NOT NULL DEFAULT 0
);
"#;
