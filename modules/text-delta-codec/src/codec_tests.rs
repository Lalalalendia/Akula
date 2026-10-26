
    fn real_like_base() -> String {
        // Deterministic incompressible *text*: the delta threshold must
        // be tested against a nontrivial full-Zstd competitor, not a tiny
        // Zstd frame for a repeated single phrase.
        let mut text = String::from("shared words\n");
        for i in 0u64..300 {
            text.push_str(&blake3::hash(&i.to_le_bytes()).to_hex().to_string());
            text.push('\n');
        }
        text
    }

    #[test]
    fn byte_exact_utf8_edits_and_full_fallback() {
        let parent_id = RevisionId::new();
        let left_id = RevisionId::new();
        let base = "Глава 1 — проверяем обычные изменения 🙂\n".repeat(80);
        for target in [
            base.replace("обычные", "очень интересные"),
            format!("{}После главы появился новый абзац.\n", base),
            String::new(),
            "Полная новая версия другого размера. Здесь новая история.".repeat(60),
        ] {
            let wire = encode(left_id, target.as_bytes(),
                              Some((parent_id, base.as_bytes()))).unwrap();
            assert_eq!(
                decode(&wire.bytes, left_id, Some((parent_id, base.as_bytes()))).unwrap(),
                target.as_bytes()
            );
            let full = encode(left_id, target.as_bytes(), None).unwrap();
            assert_eq!(full.mode, TextDeltaMode::FullZstd);
            assert_eq!(decode(&full.bytes, left_id, None).unwrap(), target.as_bytes());
            assert!(wire.bytes.len() <= full.bytes.len());
        }
    }

    #[test]
    fn independent_offline_siblings_share_only_an_exact_parent() {
        let parent_id = RevisionId::new();
        let left_id = RevisionId::new();
        let right_id = RevisionId::new();
        let parent = real_like_base();
        let left = parent.replacen("shared", "left", 1);
        let right = parent.replacen("shared", "right", 1);
        for (revision, target, wrong) in [
            (left_id, &left, &right), (right_id, &right, &left),
        ] {
            let envelope = encode(revision, target.as_bytes(),
                                  Some((parent_id, parent.as_bytes()))).unwrap();
            assert_ne!(envelope.mode, TextDeltaMode::FullZstd);
            assert_eq!(decode(&envelope.bytes, revision,
                              Some((parent_id, parent.as_bytes()))).unwrap(),
                       target.as_bytes());
            assert_eq!(decode(&envelope.bytes, revision,
                              Some((parent_id, wrong.as_bytes()))),
                       Err(TextDeltaError::WrongBase));
            assert_eq!(decode(&envelope.bytes, revision, None),
                       Err(TextDeltaError::MissingBase));
        }
    }

    #[test]
    fn reject_downgrade_spoofed_uuid_unknown_version_and_corrupt_payload() {
        let parent_id = RevisionId::new();
        let target_id = RevisionId::new();
        let base = real_like_base();
        let target = base.replace("words", "slightly different words");
        let envelope = encode(target_id, target.as_bytes(),
                              Some((parent_id, base.as_bytes()))).unwrap();
        assert_ne!(envelope.mode, TextDeltaMode::FullZstd);
        let correct_base = Some((parent_id, base.as_bytes()));
        let mut bad = envelope.bytes.clone();
        bad[0] = b'X';
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::InvalidEnvelope));
        bad = envelope.bytes.clone();
        bad[8] = 2;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::Unsupported));
        bad = envelope.bytes.clone();
        bad[9] = 255;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::Unsupported));
        bad = envelope.bytes.clone();
        bad[10] = 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::InvalidEnvelope));
        bad = envelope.bytes.clone();
        bad[28] ^= 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::WrongRevision));
        bad = envelope.bytes.clone();
        bad[44] ^= 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::WrongBase));
        bad = envelope.bytes.clone();
        bad[76] ^= 1;
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::Integrity));
        bad = envelope.bytes.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(decode(&bad,target_id,correct_base).is_err());
        bad = envelope.bytes.clone();
        bad.pop();
        assert!(decode(&bad,target_id,correct_base).is_err());
        bad = envelope.bytes.clone();
        bad.push(0);
        assert!(decode(&bad,target_id,correct_base).is_err());
        bad = envelope.bytes.clone();
        bad[112..116].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::OverBudget));
        bad = envelope.bytes.clone();
        bad[116..120].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bad,target_id,correct_base),Err(TextDeltaError::OverBudget));
        assert_eq!(decode(&envelope.bytes,RevisionId::new(),correct_base),
                   Err(TextDeltaError::WrongRevision));
        assert_eq!(decode(&envelope.bytes,target_id,
                          Some((RevisionId::new(),base.as_bytes()))),
                   Err(TextDeltaError::WrongBase));
        assert!(encode(target_id, &[0xff,0xfe], correct_base).is_err());
        assert_eq!(
            encode(target_id, target.as_bytes(), Some((target_id,base.as_bytes())))
                .err(), Some(TextDeltaError::WrongRevision)
        );
    }

    #[test]
    fn direct_dictionary_and_splice_modes_have_independent_decoders() {
        // Explicitly exercise modes 1 and 2 even when automatic selection
        // would choose a different encoding for a particular text.
        let parent_id = RevisionId::new();
        let target_id = RevisionId::new();
        let parent = real_like_base();
        let target = parent.replacen("shared", "новое слово", 1);
        let base = Some((parent_id, parent.as_bytes()));
        for mode in [TextDeltaMode::SpliceZstd, TextDeltaMode::DictionaryZstd] {
            let payload = match mode {
                TextDeltaMode::SpliceZstd => {
                    let patch = splice(parent.as_bytes(), target.as_bytes()).unwrap();
                    zstd::bulk::compress(&patch, LEVEL).unwrap()
                }
                TextDeltaMode::DictionaryZstd => {
                    let mut encoder = zstd::bulk::Compressor::with_dictionary(
                        LEVEL, parent.as_bytes(),
                    ).unwrap();
                    encoder.compress(target.as_bytes()).unwrap()
                }
                TextDeltaMode::FullZstd => unreachable!(),
            };
            let frame = to_header(mode, base, target_id, target.as_bytes(), payload)
                .unwrap();
            assert_eq!(
                decode(&frame, target_id, base).unwrap(), target.as_bytes(),
            );
            assert_eq!(
                decode(&frame, target_id, None),
                Err(TextDeltaError::MissingBase),
            );
            assert_eq!(
                decode(&frame, target_id,
                       Some((parent_id, "untrusted replacement".as_bytes()))),
                Err(TextDeltaError::WrongBase),
            );
        }
    }

    #[test]
    fn empty_long_random_repetitive_and_limit_edges() {
        let a = RevisionId::new();
        let b = RevisionId::new();
        let parent = "Русский markdown 🙂".repeat(1024);
        let target = format!("{}\n### end\n",parent);
        for bytes in [
            b"".as_slice(),
            "🙂🙂".as_bytes(),
            "changed arbitrary content".as_bytes(),
            target.as_bytes(),
        ] {
            let encoded = encode(b, bytes, Some((a, parent.as_bytes()))).unwrap();
            assert_eq!(decode(&encoded.bytes,b,Some((a,parent.as_bytes()))).unwrap(),bytes);
        }
        assert_eq!(
            encode(b,&vec![b'x';MAX_TEXT_BYTES+1],None).err(),
            Some(TextDeltaError::OverBudget)
        );
        let big = vec![b'A'; MAX_TEXT_BYTES];
        let encoded = encode(b,&big,None).unwrap();
        assert_eq!(decode(&encoded.bytes,b,None).unwrap(),big);
        let mut falsified = encoded.bytes.clone();
        falsified[108..112].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode(&falsified,b,None).is_err());
        assert!(!encoded.bytes.starts_with(b"AKPKEXP1"));
    }

    #[test]
    #[ignore = "Run only in the dedicated complete-Git-history codec A/B workflow"]
    fn real_git_markdown_pairs_numeric_ab() {
        use std::{path::Path, process::Command};

        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().parent().unwrap();
        let git = |args: &[&str]| -> Vec<u8> {
            let output = Command::new("git")
                .current_dir(repo)
                .args(args).output().expect("Git unavailable for actual history A/B");
            assert!(output.status.success(), "Git revision read failed: {:?}",
                    String::from_utf8_lossy(&output.stderr));
            output.stdout
        };
        let paths = [
            "docs/performance-roadmap.md",
            "README.ru.md",
            "docs/sync/adr-0006-cross-process-mutation-boundary.md",
            "docs/sync/adr-0005-inprocess-history-files-interlock.md",
            "aquilum-app/native/testdata/markdown-test.md",
        ];
        let mut cases = 0usize;
        let mut full_bytes = 0usize;
        let mut selected_bytes = 0usize;
        let mut full_count = 0usize;
        let mut splice_count = 0usize;
        let mut dictionary_count = 0usize;
        for path in paths {
            let raw_log = git(&["log", "--format=%H", "--", path]);
            let commits = std::str::from_utf8(&raw_log).unwrap()
                .lines().take(5).collect::<Vec<_>>();
            for pair in commits.windows(2) {
                let previous = format!("{}:{path}", pair[1]);
                let next = format!("{}:{path}", pair[0]);
                let old = git(&["show", &previous]);
                let new = git(&["show", &next]);
                if old.len() < 64 || new.len() < 64 || old == new
                    || old.len() > MAX_TEXT_BYTES || new.len() > MAX_TEXT_BYTES
                    || std::str::from_utf8(&old).is_err()
                    || std::str::from_utf8(&new).is_err()
                {
                    continue;
                }
                let base = RevisionId::new();
                let target = RevisionId::new();
                let chosen = encode(target, &new, Some((base, &old))).unwrap();
                let full = encode(target, &new, None).unwrap();
                assert_eq!(decode(&chosen.bytes, target, Some((base, &old))).unwrap(), new);
                assert_eq!(decode(&full.bytes, target, None).unwrap(), new);
                assert!(chosen.bytes.len() <= full.bytes.len());
                cases += 1;
                full_bytes += full.bytes.len();
                selected_bytes += chosen.bytes.len();
                match chosen.mode {
                    TextDeltaMode::FullZstd => full_count += 1,
                    TextDeltaMode::SpliceZstd => splice_count += 1,
                    TextDeltaMode::DictionaryZstd => dictionary_count += 1,
                }
            }
        }
        assert!(cases >= 9, "test requires checkout fetch-depth: 0");
        println!(
            "DELTA_TEXT_RUST_REAL_GIT pairs={cases} full_wire={full_bytes} selected_wire={selected_bytes} full={full_count} splice={splice_count} dictionary={dictionary_count}"
        );
    }
    #[test]
    fn forged_splice_offsets_and_trailing_bytes_refused() {
        let base=b"Unicode \xF0\x9F\x99\x82 content".to_vec();
        assert_eq!(splice_apply(&base,&[0;7]),Err(TextDeltaError::InvalidEnvelope));
        let mut invalid = Vec::new();
        invalid.extend_from_slice(&u32::MAX.to_le_bytes());
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.push(b'A');
        assert_eq!(splice_apply(&base,&invalid),Err(TextDeltaError::InvalidEnvelope));
        let mut invalid = Vec::new();
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.extend_from_slice(&1u32.to_le_bytes());
        invalid.extend_from_slice(&0u32.to_le_bytes());
        invalid.push(b'A');
        assert_eq!(splice_apply(&base,&invalid),Err(TextDeltaError::InvalidEnvelope));
    }
