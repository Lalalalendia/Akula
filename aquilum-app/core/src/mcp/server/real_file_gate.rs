// Real temporary Markdown + File Gate integration. Never touches a user vault.
use super::*;
use crate::app_core::{Core, CoreEvent};
use crate::files::document::read_file_snapshot_impl;
use crate::files::gate;
use crate::history::Source;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::Duration;

fn invoke(url: &str, tool: &str, arguments: Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let reply = ureq::post(url)
        .set("Authorization", &bearer())
        .send_string(&request.to_string())
        .expect("authorized MCP request");
    assert_eq!(reply.status(), 200);
    let message: Value =
        serde_json::from_reader(reply.into_reader()).expect("valid JSON-RPC response");
    assert_eq!(message["id"], 7);
    assert_eq!(
        message["result"]["isError"].as_bool(),
        Some(false),
        "MCP tool error: {message}"
    );
    serde_json::from_str(
        message["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text"),
    )
    .expect("structured tool result")
}

fn assert_full_revision(text: &str) {
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 4, "read returned a partial or malformed revision: {text:?}");
    assert_eq!(lines[0], "# Live");
    assert_eq!(lines[1], "");
    assert!(lines[2].starts_with("revision="), "missing version: {text:?}");
    assert_eq!(lines[2].len(), "revision=000".len());
    assert_eq!(lines[3], "END");
    assert!(text.ends_with('\n'));
}

#[test]
fn concurrent_mcp_reads_survive_real_file_gate_revisions_and_preserve_history() {
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let folder = temp.path().join("vault");
    fs::create_dir_all(&appdata).unwrap();
    fs::create_dir_all(&folder).unwrap();
    let folder = fs::canonicalize(folder).unwrap();
    let workspace = folder.to_str().unwrap().to_owned();
    let live: PathBuf = folder.join("Live.md");
    let other = folder.join("Independent.md");
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(&workspace, 1).unwrap();
    gate::create(
        &core,
        &live,
        "# Live\n\nrevision=000\nEND\n",
        Source::Agent,
        None,
    )
    .expect("create initial revision through File Gate");
    gate::create(
        &core,
        &other,
        "# Independent\n\nunchanged\n",
        Source::Agent,
        None,
    )
    .expect("create independent note");

    let serving_core = Arc::clone(&core);
    let (server, url) = start(Arc::new(move |name, arguments| {
        crate::mcp::tools::call(&serving_core, name, arguments)
    }));
    const READERS: usize = 4;
    const REVISIONS: usize = 20;
    const READS_EACH: usize = 48;

    let launch = Arc::new(Barrier::new(READERS + 2));
    let readers = (0..READERS)
        .map(|_| {
            let url = url.clone();
            let workspace = workspace.clone();
            let launch = Arc::clone(&launch);
            thread::spawn(move || {
                launch.wait();
                for attempt in 0..READS_EACH {
                    let answer = invoke(
                        &url,
                        "read_note",
                        json!({ "note": "Live.md", "workspace": workspace }),
                    );
                    assert_full_revision(answer["content"].as_str().unwrap());
                    if attempt % 8 == 0 {
                        let other = invoke(
                            &url,
                            "read_note",
                            json!({ "note": "Independent.md", "workspace": workspace }),
                        );
                        assert_eq!(
                            other["content"],
                            "# Independent\n\nunchanged\n",
                            "unrelated document changed"
                        );
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            })
        })
        .collect::<Vec<_>>();

    let writing_core = Arc::clone(&core);
    let writing_live = live.clone();
    let writing_launch = Arc::clone(&launch);
    let writer = thread::spawn(move || {
        writing_launch.wait();
        for revision in 1..=REVISIONS {
            let before = read_file_snapshot_impl(&writing_live).expect("read prior file");
            let next = format!("# Live\n\nrevision={revision:03}\nEND\n");
            gate::write(
                &writing_core,
                &writing_live,
                &next,
                Some(&before.hash),
                Source::Agent,
                None,
            )
            .expect("CAS File Gate update");
            thread::sleep(Duration::from_millis(2));
        }
    });
    launch.wait();
    writer.join().unwrap();
    for reader in readers {
        reader.join().unwrap();
    }

    let final_revision = format!("# Live\n\nrevision={REVISIONS:03}\nEND\n");
    assert_eq!(fs::read_to_string(&live).unwrap(), final_revision);
    let after_external = invoke(
        &url,
        "read_note",
        json!({ "note": "Live.md", "workspace": workspace }),
    );
    assert_eq!(after_external["content"], final_revision);

    let edited = invoke(
        &url,
        "update_note",
        json!({
            "note": "Live.md",
            "workspace": workspace,
            "mode": "append",
            "content": "MCP-final",
            "open": false,
        }),
    );
    assert_eq!(edited["changed"], true);
    let final_read = invoke(
        &url,
        "read_note",
        json!({ "note": "Live.md", "workspace": workspace }),
    );
    let disk = fs::read_to_string(&live).unwrap();
    assert_eq!(final_read["content"], disk);
    assert!(disk.contains(&final_revision));
    assert!(disk.ends_with("MCP-final"));
    assert_eq!(
        fs::read_to_string(other).unwrap(),
        "# Independent\n\nunchanged\n"
    );

    let history = invoke(
        &url,
        "note_history",
        json!({ "note": "Live.md", "workspace": workspace, "limit": 100 }),
    );
    assert!(
        history["total"].as_u64().unwrap_or(0) > 0,
        "real File Gate modifications must retain note history: {history}"
    );
    server.stop();
    core.shutdown();
}
