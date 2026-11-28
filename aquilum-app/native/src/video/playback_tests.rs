#[test]
#[ignore]
fn plays_a_real_file() {
    let path = std::path::PathBuf::from(std::env::var("AQ_VIDEO").unwrap());
    assert!(super::toggle(&path), "движок не открылся");
    assert!(super::state(&path).unwrap().playing);
    super::toggle(&path);
    assert!(!super::state(&path).unwrap().playing);
    super::toggle(&path);
    assert!(super::state(&path).unwrap().playing);
    let mut owned = super::Owned::default();
    owned.add(path.clone());
    let mut frames = 0;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(16));
        let (changed, _) = owned.tick();
        if changed && super::state(&path).and_then(|s| s.frame).is_some() {
            frames += 1;
        }
    }
    let state = super::state(&path).unwrap();
    eprintln!(
        "кадров {frames}, позиция {:.2} из {:.2}, играет {}",
        state.position, state.duration, state.playing
    );
    assert!(frames > 0);
}
