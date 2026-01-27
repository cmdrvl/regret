-- regret sqlite schema v1

CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE file (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL CHECK(length(path) <= 4096)
);

CREATE TABLE "commit" (
  sha TEXT PRIMARY KEY CHECK(length(sha) == 40),
  time_utc TEXT NOT NULL,
  subject TEXT,
  pr_number INTEGER,
  pr_source TEXT,
  patch_id BLOB,
  patch_id_rev BLOB,
  files_hash BLOB
);

CREATE TABLE fileset (
  files_hash BLOB PRIMARY KEY,
  file_count_total INTEGER NOT NULL,
  file_count_included INTEGER NOT NULL,
  blob BLOB NOT NULL
);

CREATE TABLE signal (
  id INTEGER PRIMARY KEY,
  ref TEXT NOT NULL,
  type TEXT NOT NULL CHECK(type IN ('revert','linked_fix')),
  time_utc TEXT NOT NULL,
  culprit_sha TEXT NOT NULL,
  evidence_sha TEXT NOT NULL,
  weight INTEGER NOT NULL,
  confidence REAL NOT NULL,
  time_to_regret_hours REAL NOT NULL,
  culprit_files_hash BLOB,
  evidence_files_hash BLOB
);

CREATE INDEX signal_time ON signal(time_utc);
CREATE INDEX signal_culprit ON signal(culprit_sha);
