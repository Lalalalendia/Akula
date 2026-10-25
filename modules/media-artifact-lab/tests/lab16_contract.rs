use akula_media_artifact_lab::{ArtifactStore, AudioBinding, Digest, LabError,
    SourceRef, StageKind, StageRequest, TimeBase};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Deserialize, Debug)]
struct Sample {
    name: String,
    source_sha256: String,
    stream_index: u32,
    start_pts: i64,
    time_num: i64,
    time_den: i64,
    pcm_sha256: String,
}
#[derive(Deserialize)]
struct Fixture { samples: Vec<Sample> }
fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/lab16_remux.json")).unwrap()
}
fn sample(name: &str) -> Sample {
    fixture().samples.into_iter().find(|x| x.name == name).unwrap()
}
fn source(name: &str, scope: &str) -> SourceRef {
    let s = sample(name);
    SourceRef { scope_id: scope.into(), source_id: name.into(), revision_id: "rev-1".into(),
        source_digest: Digest::parse(&format!("sha256:{}",s.source_sha256)).unwrap(),generation:1 }
}
fn binding(name: &str, scope: &str) -> AudioBinding {
    let s = sample(name);
    AudioBinding { source:source(name,scope),stream_index:s.stream_index,
        start_pts:s.start_pts,time_base:TimeBase::new(s.time_num,s.time_den).unwrap(),
        pcm_digest:Digest::parse(&format!("sha256:{}",s.pcm_sha256)).unwrap() }
}
fn stage(kind:StageKind,model:&str,version:&str,input_role:&str, input:Digest)->StageRequest {
    StageRequest {kind,code_version:version.into(),model_id:model.into(),
        params_digest:Digest::of_bytes(b"language=ja;policy=offline"),output_schema:1,
        inputs:BTreeMap::from([(input_role.into(),input)])}
}
fn asr_request(audio:&AudioBinding, version:&str)->StageRequest {
    stage(StageKind::AsrOriginal,"mock-asr-model",version,"decoded_pcm",audio.pcm_digest.clone())
}
fn trans_request(asr_digest:Digest,version:&str)->StageRequest {
    stage(StageKind::Translation,"mock-translator",version,"transcript_text",asr_digest)
}
fn register(store:&mut ArtifactStore,names:&[&str],scope:&str) {
    for name in names {store.register_source(&source(name,scope)).unwrap();}
}
fn mock_asr() -> &'static str {r#"{"language":"ja","text":"日本語の例","kind":"MOCK_NOT_INFERENCE"}"#}
fn mock_translation() -> &'static str {r#"{"language":"ru","text":"Пример на японском","kind":"MOCK_NOT_TRANSLATION"}"#}

#[test]
fn three_remuxed_files_have_distinct_container_digests() {
    let f=fixture();assert_eq!(f.samples.len(),3);
    let hashes=f.samples.iter().map(|x| x.source_sha256.as_str()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(hashes.len(),3);
}
#[test]
fn all_three_remux_versions_have_same_pcm_digest() {
    let f=fixture();let hashes=f.samples.iter().map(|x| x.pcm_sha256.as_str()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(hashes.len(),1);
}
#[test]
fn stream_index_changes_without_change_of_pcm() {
    assert_eq!(binding("source","alice").stream_index,1);
    assert_eq!(binding("reordered","alice").stream_index,0);
    assert_eq!(binding("source","alice").pcm_digest,binding("reordered","alice").pcm_digest);
}
#[test]
fn exact_shift_of_timeline_is_separate_from_pcm_identity() {
    let a=binding("source","alice");let b=binding("shifted","alice");
    assert_eq!(b.presentation_ns(0,16000).unwrap()-a.presentation_ns(0,16000).unwrap(),1_500_000_000);
    assert_ne!(a.binding_digest().unwrap(),b.binding_digest().unwrap());
    assert_eq!(a.pcm_digest,b.pcm_digest);
}
#[test]
fn signed_pts_and_subsecond_pcm_offsets_are_preserved() {
    let mut b=binding("source","alice");b.start_pts=-48000;
    assert_eq!(b.presentation_ns(8_000,16_000).unwrap(),-500_000_000);
    assert!(b.presentation_ns(-1,16000).is_err());
    assert!(b.presentation_ns(0,0).is_err());
}
#[test]
fn reject_invalid_timebase_and_digest_algorithm_confusion() {
    assert!(TimeBase::new(0,48000).is_err());assert!(TimeBase::new(1,-1).is_err());
    assert!(Digest::parse("900317a5836865002583e46637494af07750dba7158e8e02bf0f714adc65d4a2").is_err());
    assert!(Digest::parse("md5:abcdef").is_err());
}
#[test]
fn identical_audio_reuses_asr_and_translation_through_remux() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source","reordered","shifted"],"alice");
    let a=binding("source","alice");let r=asr_request(&a,"v1");
    let asr=db.publish(&a.source,&r,mock_asr()).unwrap();assert!(!asr.cached);
    let tr=trans_request(asr.content_digest.clone(),"v1");
    db.publish(&a.source,&tr,mock_translation()).unwrap();
    for name in ["reordered","shifted"] {
        let b=binding(name,"alice");
        assert_eq!(db.lookup(&b.source,&asr_request(&b,"v1")).unwrap().unwrap().content_digest,asr.content_digest);
        assert!(db.lookup(&b.source,&tr).unwrap().is_some());
    }
    assert_eq!(db.stage_count().unwrap(),2);
}
#[test]
fn remux_binds_three_timelines_without_recomputing_audio() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source","reordered","shifted"],"alice");
    let mut keys=std::collections::BTreeSet::new();
    for name in ["source","reordered","shifted"] {
        let b=binding(name,"alice");keys.insert(db.record_audio_binding(&b).unwrap());
    }
    assert_eq!(keys.len(),3);assert_eq!(db.binding_count().unwrap(),3);
}
#[test]
fn change_only_translator_preserves_asr() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");
    let a=binding("source","alice");let asr=db.publish(&a.source,&asr_request(&a,"v1"),mock_asr()).unwrap();
    let tr1=trans_request(asr.content_digest.clone(),"v1");let tr2=trans_request(asr.content_digest,"v2");
    assert!(db.publish(&a.source,&tr1,mock_translation()).is_ok());
    assert!(db.lookup(&a.source,&tr2).unwrap().is_none());
    assert!(db.publish(&a.source,&tr2,"alternative wording").is_ok());
    assert_eq!(db.stage_count().unwrap(),3);
}
#[test]
fn newer_asr_with_equal_output_stops_invalidation() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");
    let a=binding("source","alice");
    let old=db.publish(&a.source,&asr_request(&a,"v1"),mock_asr()).unwrap();
    let tr=trans_request(old.content_digest.clone(),"v1");db.publish(&a.source,&tr,mock_translation()).unwrap();
    let fresh=db.publish(&a.source,&asr_request(&a,"v2"),mock_asr()).unwrap();
    assert_ne!(old.stage_key,fresh.stage_key);assert_eq!(old.content_digest,fresh.content_digest);
    assert!(db.lookup(&a.source,&tr).unwrap().is_some());
    assert_eq!(db.stage_count().unwrap(),3);
}
#[test]
fn changed_asr_text_invalidates_dependent_translation() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");
    let a=binding("source","alice");
    let old=db.publish(&a.source,&asr_request(&a,"v1"),mock_asr()).unwrap();
    let changed=db.publish(&a.source,&asr_request(&a,"v2"),"new Japanese content").unwrap();
    assert_ne!(old.content_digest,changed.content_digest);
    assert!(db.lookup(&a.source,&trans_request(changed.content_digest,"v1")).unwrap().is_none());
}
#[test]
fn independent_visual_changes_leave_asr_alone() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");let a=binding("source","alice");
    let asr=asr_request(&a,"v1");let audio=db.publish(&a.source,&asr,mock_asr()).unwrap();
    let v1=stage(StageKind::Ocr,"japanese-cpu-v1","v1","frame",Digest::of_bytes(b"frame1"));
    let v2=stage(StageKind::Ocr,"japanese-cpu-v2","v1","frame",Digest::of_bytes(b"frame1"));
    db.publish(&a.source,&v1,"OCR1").unwrap();db.publish(&a.source,&v2,"OCR2").unwrap();
    assert_eq!(db.lookup(&a.source,&asr).unwrap().unwrap().content_digest,audio.content_digest);
}
#[test]
fn different_owners_never_share_stage_key() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");register(&mut db,&["source"],"bob");
    let a=binding("source","alice");let b=binding("source","bob");
    let one=asr_request(&a,"v1");let two=asr_request(&b,"v1");
    assert_ne!(one.stage_key("alice").unwrap(),two.stage_key("bob").unwrap());
    db.publish(&a.source,&one,mock_asr()).unwrap();
    assert!(db.lookup(&b.source,&two).unwrap().is_none());
}
#[test]
fn source_path_is_not_part_of_media_stage_key() {
    let a=binding("source","alice");
    let r=asr_request(&a,"v1");let key=r.stage_key("alice").unwrap();
    assert_eq!(key,r.stage_key("alice").unwrap());
    assert_ne!(key,r.stage_key("bob").unwrap());
}
#[test]
fn stage_key_is_independent_of_input_order() {
    let audio=Digest::of_bytes(b"audio");let image=Digest::of_bytes(b"image");
    let mut a=stage(StageKind::VisualFeatures,"v1","v1","audio",audio);
    a.inputs.insert("image".into(),image);
    let mut b=a.clone();let values=b.inputs.clone();b.inputs=BTreeMap::new();
    for (k,v) in values.into_iter().rev(){b.inputs.insert(k,v);}
    assert_eq!(a.stage_key("alice").unwrap(),b.stage_key("alice").unwrap());
}
#[test]
fn malformed_request_never_publishes() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");let a=binding("source","alice");
    let mut req=asr_request(&a,"v1");req.inputs.clear();
    assert!(db.publish(&a.source,&req,mock_asr()).is_err());
    assert_eq!(db.stage_count().unwrap(),0);
}
#[test]
fn same_key_with_divergent_outputs_fails_closed() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");let a=binding("source","alice");
    let req=asr_request(&a,"v1");db.publish(&a.source,&req,mock_asr()).unwrap();
    assert!(matches!(db.publish(&a.source,&req,"different"),Err(LabError::NonDeterministicOutput)));
    assert_eq!(db.stage_count().unwrap(),1);
}
#[test]
fn equal_stage_publication_is_idempotent() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");let a=binding("source","alice");
    let req=asr_request(&a,"v1");assert!(!db.publish(&a.source,&req,mock_asr()).unwrap().cached);
    assert!(db.publish(&a.source,&req,mock_asr()).unwrap().cached);
}
#[test]
fn source_generation_fences_old_results() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");let old=source("source","alice");
    let mut newer=old.clone();newer.generation=2;newer.revision_id="rev-2".into();newer.source_digest=Digest::of_bytes(b"new source contents");
    db.register_source(&newer).unwrap();
    assert!(matches!(db.lookup(&old,&asr_request(&binding("source","alice"),"v1")),Err(LabError::StaleSource)));
    assert!(matches!(db.record_audio_binding(&binding("source","alice")),Err(LabError::StaleSource)));
}
#[test]
fn same_source_generation_cannot_change_bytes() {
    let mut db=ArtifactStore::memory().unwrap();let old=source("source","alice");db.register_source(&old).unwrap();
    let mut bad=old.clone();bad.source_digest=Digest::of_bytes(b"unexpected bytes");
    assert!(matches!(db.register_source(&bad),Err(LabError::RevisionCollision)));
}
#[test]
fn revocation_blocks_cache_lookup_and_publish() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");let a=binding("source","alice");let req=asr_request(&a,"v1");
    db.publish(&a.source,&req,mock_asr()).unwrap();db.revoke_source(&a.source).unwrap();
    assert!(matches!(db.lookup(&a.source,&req),Err(LabError::Revoked)));
    assert!(matches!(db.publish(&a.source,&req,mock_asr()),Err(LabError::Revoked)));
    assert!(matches!(db.register_source(&a.source),Err(LabError::Revoked)));
}
#[test]
fn new_source_rebinds_and_discards_obsolete_timelines() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source"],"alice");
    let b=binding("source","alice");db.record_audio_binding(&b).unwrap();assert_eq!(db.binding_count().unwrap(),1);
    let mut newer=b.source.clone();newer.generation=2;newer.revision_id="rev-2".into();newer.source_digest=Digest::of_bytes(b"new video");
    db.register_source(&newer).unwrap();assert_eq!(db.binding_count().unwrap(),0);
}

static NEXT_DB:AtomicU64=AtomicU64::new(1);
fn scratch() -> PathBuf {
    let id=NEXT_DB.fetch_add(1,Ordering::Relaxed);
    std::env::temp_dir().join(format!("akula-media-lab17-{}-{}.db",std::process::id(),id))
}
#[test]
fn sqlite_cache_survives_restart() {
    let path=scratch();let a=binding("source","alice");let req=asr_request(&a,"v1");let digest;
    {
        let mut db=ArtifactStore::open(&path).unwrap();db.register_source(&a.source).unwrap();
        digest=db.publish(&a.source,&req,mock_asr()).unwrap().content_digest;
    }
    {
        let db=ArtifactStore::open(&path).unwrap();assert_eq!(db.stage_count().unwrap(),1);
        assert_eq!(db.lookup(&a.source,&req).unwrap().unwrap().content_digest,digest);
    }
    let _=std::fs::remove_file(path);
}
#[test]
fn source_revocation_does_not_revoke_unrelated_source_same_scope() {
    let mut db=ArtifactStore::memory().unwrap();register(&mut db,&["source","reordered"],"alice");
    let a=binding("source","alice");let b=binding("reordered","alice");let req=asr_request(&a,"v1");
    db.publish(&a.source,&req,mock_asr()).unwrap();db.revoke_source(&a.source).unwrap();
    assert!(matches!(db.lookup(&a.source,&req),Err(LabError::Revoked)));
    assert!(db.lookup(&b.source,&req).unwrap().is_some());
}
