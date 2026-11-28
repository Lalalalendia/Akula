use akula_music::TrackKey;
use akula_music::storage::{Store, StoreError, StoredOutcome, Totals};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "akula-music-{}-{}-{}",std::process::id(),stamp,NEXT.fetch_add(1,Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Store {
        Store::open(&self.0.join("history.sqlite3")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn begin(db: &mut Store, account: &str, id: &str) {
    assert!(db.begin(account,id,&TrackKey::new("yandex","track").unwrap(),180_000).unwrap());
}

#[test]
fn crossfade_nested_intervals_and_window() {
    let f = Fixture::new();
    let mut db = f.open();
    for (id,lo,hi) in [("a",0,10_000),("b",8_000,16_000),("c",1_000,2_000)] {
        begin(&mut db,"owner",id);
        assert!(db.checkpoint("owner",id,1,lo,hi).unwrap());
    }
    assert_eq!(db.totals("owner",0,20_000).unwrap(),Totals{wall_ms:16_000,track_audio_ms:19_000});
    assert_eq!(db.totals("owner",9_000,12_000).unwrap(),Totals{wall_ms:3_000,track_audio_ms:4_000});
}

#[test]
fn pause_and_midnight_are_not_invented() {
    let f = Fixture::new();
    let mut db = f.open();
    begin(&mut db,"owner","a");
    let day = 86_400_000;
    db.checkpoint("owner","a",1,day-5_000,day+3_000).unwrap();
    db.checkpoint("owner","a",2,day+9_000,day+10_000).unwrap();
    assert_eq!(db.totals("owner",0,day).unwrap().wall_ms,5_000);
    assert_eq!(db.totals("owner",day,day*2).unwrap().wall_ms,4_000);
}

#[test]
fn duplicate_conflict_and_order() {
    let f = Fixture::new();
    let mut db = f.open();
    begin(&mut db,"owner","a");
    assert!(!db.begin("owner","a",&TrackKey::new("yandex","track").unwrap(),180_000).unwrap());
    assert!(matches!(db.begin("owner","a",&TrackKey::new("yandex","other").unwrap(),180_000),Err(StoreError::Conflict)));
    db.checkpoint("owner","a",1,0,1_000).unwrap();
    assert!(!db.checkpoint("owner","a",1,0,1_000).unwrap());
    assert!(matches!(db.checkpoint("owner","a",1,0,2_000),Err(StoreError::Conflict)));
    assert!(db.checkpoint("owner","a",3,1_000,2_000).is_err());
    assert!(db.checkpoint("owner","a",2,500,2_000).is_err());
    assert!(db.checkpoint("owner","a",2,1_000,2_000).unwrap());
    assert_eq!(db.totals("owner",0,5_000).unwrap().wall_ms,2_000);
}

#[test]
fn reopen_preserves_committed_progress_and_only_explicit_recovery_closes() {
    let f = Fixture::new();
    {
        let mut db = f.open();
        begin(&mut db,"owner","a");
        begin(&mut db,"owner","b");
        db.checkpoint("owner","a",1,0,1_000).unwrap();
    }
    let mut db = f.open();
    assert_eq!(db.totals("owner",0,10_000).unwrap().wall_ms,1_000);
    assert!(db.finish("owner","a",StoredOutcome::Interrupted).unwrap());
    assert!(!db.finish("owner","a",StoredOutcome::Interrupted).unwrap());
    assert!(db.finish("owner","a",StoredOutcome::NaturalEnd).is_err());
    assert!(!db.checkpoint("owner","a",1,0,1_000).unwrap());
    assert!(db.checkpoint("owner","a",2,1_000,2_000).is_err());
    assert!(db.checkpoint("owner","b",1,3_000,4_000).unwrap());
    assert_eq!(db.totals("owner",0,10_000).unwrap().wall_ms,2_000);
}

#[test]
fn accounts_and_deletion_are_isolated() {
    let f = Fixture::new();
    let mut db = f.open();
    begin(&mut db,"owner","a");
    begin(&mut db,"other","b");
    db.checkpoint("owner","a",1,0,10_000).unwrap();
    db.checkpoint("other","b",1,0,20_000).unwrap();
    assert!(matches!(db.checkpoint("other","a",2,10_000,11_000),Err(StoreError::UnknownSession)));
    assert!(matches!(db.finish("other","a",StoredOutcome::Stopped),Err(StoreError::UnknownSession)));
    assert!(matches!(db.clear_account("owner"),Err(StoreError::ActiveSession)));
    db.finish("owner","a",StoredOutcome::Stopped).unwrap();
    assert_eq!(db.clear_account("owner").unwrap(),1);
    assert_eq!(db.totals("owner",0,30_000).unwrap(),Totals::default());
    assert_eq!(db.totals("other",0,30_000).unwrap().wall_ms,20_000);
}

#[test]
fn two_connections_retry_without_duplicate_time() {
    let f = Fixture::new();
    let mut a = f.open();
    let mut b = f.open();
    begin(&mut a,"owner","a");
    a.checkpoint("owner","a",1,0,1_000).unwrap();
    assert!(!b.checkpoint("owner","a",1,0,1_000).unwrap());
    assert_eq!(b.totals("owner",0,10_000).unwrap().wall_ms,1_000);
}

#[test]
fn rejects_foreign_database_and_unknown_schema() {
    let f = Fixture::new();
    let path = f.0.join("foreign.sqlite3");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("CREATE TABLE unrelated (id INTEGER)",[]).unwrap();
    drop(conn);
    assert!(matches!(Store::open(&path),Err(StoreError::ForeignDatabase)));
    let db = f.open();
    drop(db);
    let path = f.0.join("history.sqlite3");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None,"user_version",2).unwrap();
    drop(conn);
    assert!(matches!(Store::open(&path),Err(StoreError::ForeignDatabase)));
}

#[test]
fn invalid_inputs_fail_without_writes() {
    let f = Fixture::new();
    let mut db = f.open();
    begin(&mut db,"owner","a");
    assert!(db.checkpoint("owner","a",0,0,100).is_err());
    assert!(db.checkpoint("owner","a",1,-1,100).is_err());
    assert!(db.checkpoint("owner","a",1,100,100).is_err());
    assert!(db.totals("owner",1,1).is_err());
    assert_eq!(db.totals("owner",0,1_000).unwrap(),Totals::default());
}
