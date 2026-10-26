//! Opt-in Notion laboratory CLI; NEVER a public login flow or background agent.
//! --cache writes Akula metadata; --publish-cached reads only a verified API
//! snapshot and explicitly uses create-only File Gate on a registered Vault.
use aquilum_core::app_core::{Core, CoreEvent};
use aquilum_core::notion_api_probe::{
    cached_import::{approved_cached_preview, read_verified_cached},
    http::HttpReadClient, inspect_root, projection::prepare_import,
    publish::{publish_approved, PublishReport}, refresh::refresh_root,
    snapshot::SnapshotStore, ProbeLimits,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};
use uuid::Uuid;

enum Mode {
    Preview,
    Cache(PathBuf),
    Offline(PathBuf),
    CachedPreview(PathBuf),
    Publish(PathBuf, PathBuf),
    PublishCached(PathBuf, PathBuf, String),
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Notion import refused: {e}");
        std::process::exit(1);
    }
}

fn app_data(path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| "existing Akula app data directory is required")?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err("Akula app data must be a real directory, not a symlink".into());
    }
    fs::canonicalize(path).map_err(|_| "cannot resolve Akula app data path".into())
}

fn print_publish(report: PublishReport) -> Result<(), String> {
    println!("{}", serde_json::to_string_pretty(&report)
        .map_err(|_| "cannot encode Notion import report")?);
    if !report.completed {
        return Err("import incomplete; inspect report".into());
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let usage = "usage: --preview ROOT | --cache ROOT APP_DATA --approve-cache \
        | --offline ROOT APP_DATA | --cached-preview ROOT APP_DATA \
        | --publish-cached ROOT APP_DATA REGISTERED_WORKSPACE DIGEST --approve-create-only \
        | --publish ROOT APP_DATA REGISTERED_WORKSPACE --approve-create-only";
    let (root, mode) = match args.as_slice() {
        [cmd, root] if cmd == "--preview" =>
            (root, Mode::Preview),
        [cmd, root, app, approval] if cmd == "--cache" && approval == "--approve-cache" =>
            (root, Mode::Cache(PathBuf::from(app))),
        [cmd, root, app] if cmd == "--offline" =>
            (root, Mode::Offline(PathBuf::from(app))),
        [cmd, root, app] if cmd == "--cached-preview" =>
            (root, Mode::CachedPreview(PathBuf::from(app))),
        [cmd, root, app, workspace, digest, approval]
            if cmd == "--publish-cached" && approval == "--approve-create-only" =>
            (root, Mode::PublishCached(PathBuf::from(app),
                PathBuf::from(workspace), digest.clone())),
        [cmd, root, app, workspace, approval]
            if cmd == "--publish" && approval == "--approve-create-only" =>
            (root, Mode::Publish(PathBuf::from(app), PathBuf::from(workspace))),
        _ => return Err(usage.into()),
    };
    let root = Uuid::parse_str(root)
        .map_err(|_| "invalid Notion root UUID")?.to_string();

    // All three cache-only paths are processed before we even read an env token.
    // This works after revocation or when entirely disconnected from Notion.
    if let Mode::Offline(app) = &mode {
        let app = app_data(app)?;
        let store = SnapshotStore::open_read_only(&app.join("ui-state.sqlite3"))?;
        let cached = store.load(&root)?
            .ok_or("Notion root not cached; an online first read is required")?;
        let links = cached.info.link_count;
        let imported = prepare_import(&cached.into_preview())?;
        println!("offline=true notes={} links={} can_import={}",
            imported.candidates.len(), links, imported.publish_allowed);
        return Ok(());
    }
    if let Mode::CachedPreview(app) = &mode {
        let app = app_data(app)?;
        let (review, _) = read_verified_cached(&app.join("ui-state.sqlite3"), &root)?;
        println!("{}", serde_json::to_string_pretty(&review)
            .map_err(|_| "cannot encode cached import review")?);
        return Ok(());
    }
    if let Mode::PublishCached(app, workspace, digest) = &mode {
        let app = app_data(app)?;
        let preview = approved_cached_preview(&app.join("ui-state.sqlite3"), &root, digest)?;
        let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
        return print_publish(publish_approved(&core, workspace, &preview, true)?);
    }

    let token = env::var("AKULA_NOTION_READ_TOKEN")
        .map_err(|_| "Notion token not configured locally")?;
    let mut client = HttpReadClient::new(token)
        .map_err(|_| "invalid Notion credentials")?;
    if let Mode::Cache(app) = &mode {
        let app = app_data(app)?;
        let mut store = SnapshotStore::open(&app.join("ui-state.sqlite3"))?;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(|_| "invalid system time")?.as_millis();
        let timestamp = i64::try_from(timestamp)
            .map_err(|_| "system time outside SQLite range")?;
        let receipt = refresh_root(&mut client, &mut store,
            &root, ProbeLimits::default(), timestamp)?;
        println!("cached=true change={:?} objects={} links={} digest={}",
            receipt.change, receipt.snapshot.object_count, receipt.snapshot.link_count,
            receipt.snapshot.digest);
        return Ok(());
    }
    let read = inspect_root(&mut client, &root, ProbeLimits::default())?;
    let preview = prepare_import(&read)?;
    println!("objects={} unresolved={} can_publish={}",
        preview.candidates.len(), preview.unresolved.len(), preview.publish_allowed);

    match mode {
        Mode::Preview => Ok(()),
        Mode::Cache(_) | Mode::Offline(_) | Mode::CachedPreview(_)
            | Mode::PublishCached(_, _, _) =>
            unreachable!("already handled without a repeated API read"),
        Mode::Publish(app, workspace) => {
            let app = app_data(&app)?;
            let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
            print_publish(publish_approved(&core, &workspace, &preview, true)?)
        }
    }
}
