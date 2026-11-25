CREATE TABLE creator_project_metadata (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES creator_program_projects(project_id),
    modified_at INTEGER NOT NULL CHECK(modified_at BETWEEN 0 AND 9007199254740991)
);
INSERT INTO creator_project_metadata SELECT project_id, CAST((julianday('now')-2440587.5)*86400000 AS INTEGER) FROM creator_program_projects;
CREATE TRIGGER creator_project_insert_time AFTER INSERT ON creator_program_projects BEGIN
    INSERT INTO creator_project_metadata VALUES(NEW.project_id, CAST((julianday('now')-2440587.5)*86400000 AS INTEGER));
END;
CREATE TRIGGER creator_project_update_time AFTER UPDATE ON creator_program_projects BEGIN
    UPDATE creator_project_metadata SET modified_at=MAX(modified_at,CAST((julianday('now')-2440587.5)*86400000 AS INTEGER)) WHERE project_id=NEW.project_id;
END;
CREATE TABLE creator_jobs (
    job_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES creator_program_projects(project_id),
    project_revision INTEGER NOT NULL CHECK(project_revision BETWEEN 0 AND 9007199254740991),
    workspace TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued','running','cancelling','cancelled','interrupted','completed','failed')),
    run_token TEXT NOT NULL,
    state_json BLOB NOT NULL,
    request_json BLOB NOT NULL,
    stamp_json BLOB NOT NULL
);
CREATE UNIQUE INDEX creator_one_workspace_writer ON creator_jobs(workspace) WHERE status IN ('queued','running','cancelling');
CREATE TABLE creator_checkpoint_results (
    checkpoint_id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL REFERENCES creator_jobs(job_id),
    evidence BLOB NOT NULL CHECK(length(evidence) <= 2097152)
);
CREATE TABLE creator_releases (
    release_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES creator_program_projects(project_id),
    project_revision INTEGER NOT NULL CHECK(project_revision BETWEEN 0 AND 9007199254740991),
    release_json BLOB NOT NULL
);
CREATE TABLE creator_reports (
    report_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES creator_program_projects(project_id),
    project_revision INTEGER NOT NULL CHECK(project_revision BETWEEN 0 AND 9007199254740991),
    report_json BLOB NOT NULL
);
CREATE TRIGGER creator_release_no_update BEFORE UPDATE ON creator_releases BEGIN SELECT RAISE(ABORT,'Immutable Creator release'); END;
CREATE TRIGGER creator_release_no_delete BEFORE DELETE ON creator_releases BEGIN SELECT RAISE(ABORT,'Immutable Creator release'); END;
CREATE TRIGGER creator_report_no_update BEFORE UPDATE ON creator_reports BEGIN SELECT RAISE(ABORT,'Immutable Creator report'); END;
CREATE TRIGGER creator_report_no_delete BEFORE DELETE ON creator_reports BEGIN SELECT RAISE(ABORT,'Immutable Creator report'); END;
CREATE TRIGGER creator_checkpoint_no_update BEFORE UPDATE ON creator_checkpoint_results BEGIN SELECT RAISE(ABORT,'Immutable Creator checkpoint'); END;
CREATE TRIGGER creator_checkpoint_no_delete BEFORE DELETE ON creator_checkpoint_results BEGIN SELECT RAISE(ABORT,'Immutable Creator checkpoint'); END;
PRAGMA user_version=7;
