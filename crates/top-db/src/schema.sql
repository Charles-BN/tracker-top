-- Master pile database.
--
-- piles        the immutable core: one row per pile of the original pile list.
--              Rows can be added (a later master list with new piles) but never
--              changed or deleted; triggers enforce it.
-- imports      every file ever ingested (master list, field survey, LIDAR).
--              Append-only, except that a survey/LIDAR import can be voided once.
-- elevations   ground elevations read from survey / LIDAR files. Append-only.
-- flood_values flood value per pile (depth above ground or water elevation,
--              see the flood_mode setting). Append-only.
-- runs         optimisation runs; run_trackers / run_piles hold their results.
--              Append-only. The current result of a tracker is its latest run.

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS imports (
    import_id     INTEGER PRIMARY KEY,
    kind          TEXT NOT NULL CHECK (kind IN ('master', 'survey', 'lidar', 'flood')),
    file_name     TEXT NOT NULL,
    sha256        TEXT NOT NULL,
    imported_at   TEXT NOT NULL,
    survey_date   TEXT,
    origin        TEXT NOT NULL,
    rows_total    INTEGER NOT NULL,
    rows_used     INTEGER NOT NULL,
    rows_rejected INTEGER NOT NULL,
    report        TEXT NOT NULL,
    voided_at     TEXT,
    void_reason   TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS imports_unique_file
    ON imports (kind, sha256) WHERE voided_at IS NULL;

CREATE TRIGGER IF NOT EXISTS imports_no_delete BEFORE DELETE ON imports
BEGIN
    SELECT RAISE(ABORT, 'imports are permanent; void them instead');
END;

CREATE TRIGGER IF NOT EXISTS imports_void_only BEFORE UPDATE ON imports
WHEN OLD.voided_at IS NOT NULL
  OR NEW.voided_at IS NULL
  OR OLD.kind = 'master'
  OR NEW.import_id IS NOT OLD.import_id
  OR NEW.kind IS NOT OLD.kind
  OR NEW.file_name IS NOT OLD.file_name
  OR NEW.sha256 IS NOT OLD.sha256
  OR NEW.imported_at IS NOT OLD.imported_at
  OR NEW.survey_date IS NOT OLD.survey_date
  OR NEW.origin IS NOT OLD.origin
  OR NEW.rows_total IS NOT OLD.rows_total
  OR NEW.rows_used IS NOT OLD.rows_used
  OR NEW.rows_rejected IS NOT OLD.rows_rejected
  OR NEW.report IS NOT OLD.report
BEGIN
    SELECT RAISE(ABORT, 'an import can only be voided, once, and master imports never');
END;

CREATE TABLE IF NOT EXISTS piles (
    pile_id   TEXT PRIMARY KEY,
    tracker   TEXT NOT NULL,
    block     TEXT NOT NULL DEFAULT '',
    section   TEXT NOT NULL DEFAULT '',
    easting   REAL NOT NULL,
    northing  REAL NOT NULL,
    pile_type TEXT NOT NULL DEFAULT '',
    color     TEXT NOT NULL DEFAULT '',
    tracker_type TEXT NOT NULL DEFAULT '',
    import_id INTEGER NOT NULL REFERENCES imports (import_id)
);
CREATE INDEX IF NOT EXISTS piles_tracker ON piles (tracker);

CREATE TRIGGER IF NOT EXISTS piles_immutable_update BEFORE UPDATE ON piles
BEGIN
    SELECT RAISE(ABORT, 'the master pile list is immutable');
END;
CREATE TRIGGER IF NOT EXISTS piles_immutable_delete BEFORE DELETE ON piles
BEGIN
    SELECT RAISE(ABORT, 'the master pile list is immutable');
END;

CREATE TABLE IF NOT EXISTS elevations (
    import_id  INTEGER NOT NULL REFERENCES imports (import_id),
    pile_id    TEXT NOT NULL REFERENCES piles (pile_id),
    z          REAL NOT NULL,
    easting    REAL,
    northing   REAL,
    offset_m   REAL,
    matched_by TEXT NOT NULL CHECK (matched_by IN ('id', 'position')),
    code       TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (import_id, pile_id)
);
CREATE INDEX IF NOT EXISTS elevations_pile ON elevations (pile_id);

CREATE TRIGGER IF NOT EXISTS elevations_no_update BEFORE UPDATE ON elevations
BEGIN
    SELECT RAISE(ABORT, 'elevations are append-only');
END;
CREATE TRIGGER IF NOT EXISTS elevations_no_delete BEFORE DELETE ON elevations
BEGIN
    SELECT RAISE(ABORT, 'elevations are append-only');
END;

CREATE TABLE IF NOT EXISTS flood_values (
    import_id  INTEGER NOT NULL REFERENCES imports (import_id),
    pile_id    TEXT NOT NULL REFERENCES piles (pile_id),
    value      REAL NOT NULL,
    matched_by TEXT NOT NULL CHECK (matched_by IN ('id', 'position')),
    PRIMARY KEY (import_id, pile_id)
);
CREATE INDEX IF NOT EXISTS flood_values_pile ON flood_values (pile_id);

CREATE TRIGGER IF NOT EXISTS flood_values_no_update BEFORE UPDATE ON flood_values
BEGIN
    SELECT RAISE(ABORT, 'flood values are append-only');
END;
CREATE TRIGGER IF NOT EXISTS flood_values_no_delete BEFORE DELETE ON flood_values
BEGIN
    SELECT RAISE(ABORT, 'flood values are append-only');
END;

CREATE TABLE IF NOT EXISTS runs (
    run_id          INTEGER PRIMARY KEY,
    created_at      TEXT NOT NULL,
    params          TEXT NOT NULL,
    trigger         TEXT NOT NULL,
    trackers_run    INTEGER NOT NULL,
    duration_ms     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS run_trackers (
    run_id        INTEGER NOT NULL REFERENCES runs (run_id),
    tracker       TEXT NOT NULL,
    status        TEXT NOT NULL,
    input_sig     TEXT NOT NULL,
    n_survey      INTEGER NOT NULL,
    n_lidar       INTEGER NOT NULL,
    max_angle_deg REAL NOT NULL,
    max_cut_fill  REAL NOT NULL,
    PRIMARY KEY (run_id, tracker)
);
CREATE INDEX IF NOT EXISTS run_trackers_tracker ON run_trackers (tracker, run_id);

CREATE TABLE IF NOT EXISTS run_piles (
    run_id         INTEGER NOT NULL REFERENCES runs (run_id),
    pile_id        TEXT NOT NULL REFERENCES piles (pile_id),
    z_ground       REAL NOT NULL,
    z_import_id    INTEGER NOT NULL REFERENCES imports (import_id),
    new_ground     REAL NOT NULL,
    cut_fill       REAL NOT NULL,
    top            REAL NOT NULL,
    reveal         REAL NOT NULL,
    angle_diff_deg REAL NOT NULL,
    outlier        INTEGER NOT NULL,
    min_top        REAL,
    min_top_active INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, pile_id)
);

CREATE TRIGGER IF NOT EXISTS runs_no_update BEFORE UPDATE ON runs
BEGIN SELECT RAISE(ABORT, 'optimisation runs are append-only'); END;
CREATE TRIGGER IF NOT EXISTS runs_no_delete BEFORE DELETE ON runs
BEGIN SELECT RAISE(ABORT, 'optimisation runs are append-only'); END;
CREATE TRIGGER IF NOT EXISTS run_trackers_no_update BEFORE UPDATE ON run_trackers
BEGIN SELECT RAISE(ABORT, 'optimisation runs are append-only'); END;
CREATE TRIGGER IF NOT EXISTS run_trackers_no_delete BEFORE DELETE ON run_trackers
BEGIN SELECT RAISE(ABORT, 'optimisation runs are append-only'); END;
CREATE TRIGGER IF NOT EXISTS run_piles_no_update BEFORE UPDATE ON run_piles
BEGIN SELECT RAISE(ABORT, 'optimisation runs are append-only'); END;
CREATE TRIGGER IF NOT EXISTS run_piles_no_delete BEFORE DELETE ON run_piles
BEGIN SELECT RAISE(ABORT, 'optimisation runs are append-only'); END;
