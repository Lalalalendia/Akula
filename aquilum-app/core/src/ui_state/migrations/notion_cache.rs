//! v9: selected external Notion roots live in the existing UI-state SQLite file.
//! No access tokens or user-owned vault bytes are stored here.
use super::super::error::UiStateError;
use rusqlite::Connection;

pub const NOTION_SCHEMA: &str = "
    CREATE TABLE notion_cache_sources (
        root_id TEXT PRIMARY KEY,
        generation TEXT NOT NULL,
        api_version TEXT NOT NULL,
        checksum TEXT NOT NULL,
        saved_at_ms INTEGER NOT NULL,
        object_count INTEGER NOT NULL CHECK (object_count >= 0),
        link_count INTEGER NOT NULL CHECK (link_count >= 0)
    );
    CREATE TABLE notion_cache_objects (
        root_id TEXT NOT NULL REFERENCES notion_cache_sources(root_id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        object_id TEXT NOT NULL,
        raw_json TEXT NOT NULL,
        PRIMARY KEY(root_id, kind, object_id)
    );
    CREATE TABLE notion_cache_edges (
        root_id TEXT NOT NULL REFERENCES notion_cache_sources(root_id) ON DELETE CASCADE,
        source_id TEXT NOT NULL,
        target_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        property_id TEXT NOT NULL DEFAULT '',
        PRIMARY KEY(root_id, source_id, target_id, kind, property_id)
    );
    CREATE INDEX notion_cache_backlinks
        ON notion_cache_edges(root_id, target_id, kind);
";

pub fn migrate_v8_to_v9(connection: &Connection) -> Result<(), UiStateError> {
    connection.execute_batch(&format!(
        "BEGIN IMMEDIATE; {NOTION_SCHEMA} PRAGMA user_version = 9; COMMIT;"
    ))?;
    Ok(())
}
