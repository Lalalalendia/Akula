use super::*;

#[test]
fn decoded_cues_use_the_media_timeline_and_clamp_model_offsets() {
    let json = br#"{"transcription":[
        {"offsets":{"from":900,"to":1200},"text":" after"},
        {"offsets":{"from":-60,"to":400},"text":" before "},
        {"offsets":{"from":1100,"to":1600},"text":"outside"},
        {"offsets":{"from":550,"to":600},"text":" "}
    ]}"#;
    assert_eq!(
        output::decode_whisper(json, 30_000..31_000).unwrap(),
        [
            Cue {
                start_ms: 30_000,
                end_ms: 30_400,
                text: "before".into()
            },
            Cue {
                start_ms: 30_900,
                end_ms: 31_000,
                text: "after".into()
            },
        ]
    );
}

#[test]
fn cancelled_jobs_keep_committed_cues_and_export_plain_vtt() {
    let temp = tempfile::tempdir().unwrap();
    let mut queue = Queue::open(&temp.path().join("asr.sqlite")).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: KIND.into(),
            source: "source".into(),
            revision: "a".repeat(64),
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue.claim(KIND, Some(id), LEASE).unwrap().unwrap();
    queue.set_total(&lease, 2).unwrap();
    let cue = Cue {
        start_ms: 1_005,
        end_ms: 3_100,
        text: " \n<c>Original & speech</c>\r\n".into(),
    };
    queue
        .append(
            &lease,
            &Segment {
                ordinal: 0,
                start_ms: 0,
                end_ms: WINDOW_MS,
                payload: serde_json::to_vec(&[&cue]).unwrap(),
            },
        )
        .unwrap();
    queue.cancel(id).unwrap();
    assert_eq!(read_cues(&queue, id, 0, 16).unwrap(), [(0, vec![cue])]);
    let mut vtt = Vec::new();
    write_vtt(&queue, id, &mut vtt).unwrap();
    assert_eq!(
        String::from_utf8(vtt).unwrap(),
        "WEBVTT\n\n00:00:01.005 --> 00:00:03.100\n&lt;c&gt;Original &amp; speech&lt;/c&gt;\n\n"
    );
}
