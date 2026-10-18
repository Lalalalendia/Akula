use super::store;
use std::borrow::Cow;
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

const IDLE: Duration = Duration::from_secs(60);
const CAP: Duration = Duration::from_secs(5 * 60);
const BIG_CHANGE_CHARS: usize = 200;
const RESTORE_LABEL: &str = "restore";
const REVERT_LABEL: &str = "revert";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Me,
    Agent,
    External,
    Links,
    Start,
    Reading,
    Restore(u64),
    Revert(u64),
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Agent => "agent",
            Self::External => "external",
            Self::Links => "links",
            Self::Start => "start",
            Self::Reading => "reading",
            Self::Restore(_) => RESTORE_LABEL,
            Self::Revert(_) => REVERT_LABEL,
        }
    }

    pub fn file_label(self) -> String {
        match self.origin_ms() {
            Some(from_ms) => format!("{}-{from_ms:013}", self.label()),
            None => self.label().to_owned(),
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        if let Some((kind, from_ms)) = label.split_once('-') {
            let from_ms = from_ms.parse().ok()?;
            return match kind {
                RESTORE_LABEL => Some(Self::Restore(from_ms)),
                REVERT_LABEL => Some(Self::Revert(from_ms)),
                _ => None,
            };
        }
        [Self::Me, Self::Agent, Self::External, Self::Links, Self::Start]
            .into_iter()
            .find(|source| source.label() == label)
    }

    pub fn origin_ms(self) -> Option<u64> {
        match self {
            Self::Restore(from_ms) | Self::Revert(from_ms) => Some(from_ms),
            _ => None,
        }
    }

    fn is_quiet(self) -> bool {
        matches!(self, Self::Reading)
    }
}

impl Serialize for Source {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot<'a> {
    // Borrow the already immutable save buffer; never duplicate a large note
    // merely to decide/persist a history milestone.
    pub text: Cow<'a, str>,
    pub source: Source,
    pub at: SystemTime,
}

pub struct Before<'a> {
    pub text: &'a str,
    pub at: SystemTime,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MilestonePlan<'a> {
    Quiet,
    Skip,
    RevertToPrevious {
        remove_id: String,
    },
    UpdateLatest {
        id: String,
    },
    CreateNew {
        baseline: Option<Snapshot<'a>>,
        snapshot: Snapshot<'a>,
    },
}

pub struct Incoming<'a> {
    pub source: Source,
    pub text: &'a str,
    pub disk_before: Option<&'a Before<'a>>,
}

pub struct Recorded<'a> {
    pub versions: &'a [store::VersionFile],
    pub names: &'a HashMap<String, String>,
    pub latest_text: Option<&'a str>,
    pub previous_text: Option<&'a str>,
    pub last_edit: Option<SystemTime>,
}

pub fn plan<'a>(
    device: &str,
    incoming: &Incoming<'a>,
    recorded: &Recorded<'_>,
    now: SystemTime,
) -> MilestonePlan<'a> {
    let Incoming {
        source: write_source,
        text: write_text,
        disk_before,
    } = *incoming;
    let Recorded {
        versions,
        names,
        latest_text,
        previous_text,
        last_edit,
    } = *recorded;
    if write_source.is_quiet() {
        return MilestonePlan::Quiet;
    }

    let latest = versions.last();
    let has_versions = latest.is_some();

    if let (Some(latest), Some(text)) = (latest, latest_text) {
        if text == write_text {
            return MilestonePlan::Skip;
        }

        if write_source == Source::Me
            && versions.len() >= 2
            && latest.source == Source::Me
            && latest.device == device
            && !names.contains_key(&latest.id)
            && previous_text.is_some_and(|prev| prev == write_text)
        {
            return MilestonePlan::RevertToPrevious {
                remove_id: latest.id.clone(),
            };
        }
    }

    let baseline = if !has_versions {
        disk_before.and_then(|disk| {
            (!disk.text.is_empty() && disk.text != write_text).then(|| Snapshot {
                text: Cow::Borrowed(disk.text),
                source: Source::Start,
                at: disk.at,
            })
        })
    } else {
        None
    };

    let can_coalesce = latest.is_some_and(|latest| {
        write_source == Source::Me
            && latest.source == Source::Me
            && latest.device == device
            && !names.contains_key(&latest.id)
            && last_edit.is_some_and(|le| elapsed(le, now) < IDLE)
            && elapsed(store::system_time_from_ms(latest.at_ms), now) < CAP
            && latest_text.is_some_and(|text| changed_chars(text, write_text) < BIG_CHANGE_CHARS)
    });

    if can_coalesce {
        let latest = latest.unwrap();
        MilestonePlan::UpdateLatest {
            id: latest.id.clone(),
        }
    } else {
        MilestonePlan::CreateNew {
            baseline,
            snapshot: Snapshot {
                text: Cow::Borrowed(write_text),
                source: write_source,
                at: now,
            },
        }
    }
}

fn changed_chars(before: &str, after: &str) -> usize {
    let prefix = common_prefix(before, after);
    let (before, after) = (&before[prefix..], &after[prefix..]);
    let suffix = common_suffix(before, after);
    let changed = |text: &str| text[..text.len() - suffix].chars().count();
    changed(before).max(changed(after))
}

fn common_prefix(left: &str, right: &str) -> usize {
    let mut length = left
        .bytes()
        .zip(right.bytes())
        .take_while(|(left, right)| left == right)
        .count();
    while !left.is_char_boundary(length) || !right.is_char_boundary(length) {
        length -= 1;
    }
    length
}

fn common_suffix(left: &str, right: &str) -> usize {
    let mut length = left
        .bytes()
        .rev()
        .zip(right.bytes().rev())
        .take_while(|(left, right)| left == right)
        .count();
    while !left.is_char_boundary(left.len() - length) || !right.is_char_boundary(right.len() - length) {
        length -= 1;
    }
    length
}

fn elapsed(from: SystemTime, to: SystemTime) -> Duration {
    to.duration_since(from).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        changed_chars, plan, Before, Incoming, MilestonePlan, Recorded, Snapshot, Source,
        BIG_CHANGE_CHARS, CAP, IDLE,
    };
    use crate::history::store::VersionFile;
    use std::collections::HashMap;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    fn version_file(id: &str, seconds: u64, source: Source, device: &str) -> VersionFile {
        VersionFile {
            id: id.to_owned(),
            at_ms: (1_800_000_000 + seconds) * 1000,
            device: device.to_owned(),
            source,
            from_ms: source.origin_ms(),
            name: None,
            is_current: false,
        }
    }

    #[test]
    fn first_edit_creates_baseline_and_new_version() {
        let before = Before {
            text: "исходный",
            at: at(10),
        };
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: "исходный + правка",
                disk_before: Some(&before),
            },
            &Recorded {
                versions: &[],
                names: &HashMap::new(),
                latest_text: None,
                previous_text: None,
                last_edit: None,
            },
            at(20),
        );
        assert_eq!(
            p,
            MilestonePlan::CreateNew {
                baseline: Some(Snapshot {
                    text: "исходный".to_owned().into(),
                    source: Source::Start,
                    at: at(10)
                }),
                snapshot: Snapshot {
                    text: "исходный + правка".to_owned().into(),
                    source: Source::Me,
                    at: at(20)
                }
            }
        );
    }

    #[test]
    fn creating_history_versions_borrows_existing_large_source_buffers() {
        use std::borrow::Cow;

        let current = "Текущий текст 😀漢字\n".repeat(100_000);
        let earlier = "Предыдущая версия\r\n".repeat(100_000);
        let original = Before { text: &earlier, at: at(10) };
        let incoming = Incoming {
            source: Source::Me,
            text: &current,
            disk_before: Some(&original),
        };
        let empty_versions = [];
        let names = HashMap::new();
        let recorded = Recorded {
            versions: &empty_versions,
            names: &names,
            latest_text: None,
            previous_text: None,
            last_edit: None,
        };

        let planned = plan("dev1", &incoming, &recorded, at(20));
        let MilestonePlan::CreateNew { baseline: Some(baseline), snapshot } = planned else {
            panic!("an initial edit must preserve a baseline and new snapshot");
        };
        assert!(matches!(&baseline.text, Cow::Borrowed(_)));
        assert!(matches!(&snapshot.text, Cow::Borrowed(_)));
        assert_eq!(baseline.text.as_bytes().as_ptr(), earlier.as_bytes().as_ptr());
        assert_eq!(snapshot.text.as_bytes().as_ptr(), current.as_bytes().as_ptr());
        assert_eq!(baseline.text.as_ref(), earlier);
        assert_eq!(snapshot.text.as_ref(), current);
    }

    #[test]
    fn subsequent_typing_within_idle_coalesces() {
        let v = version_file("v1.md", 20, Source::Me, "dev1");
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: "исходный + правка ещё",
                disk_before: None,
            },
            &Recorded {
                versions: &[v],
                names: &HashMap::new(),
                latest_text: Some("исходный + правка"),
                previous_text: None,
                last_edit: Some(at(20)),
            },
            at(25),
        );
        assert_eq!(
            p,
            MilestonePlan::UpdateLatest {
                id: "v1.md".to_owned()
            }
        );
    }

    #[test]
    fn typing_after_idle_starts_new_version() {
        let v = version_file("v1.md", 20, Source::Me, "dev1");
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: "новая мысль",
                disk_before: None,
            },
            &Recorded {
                versions: &[v],
                names: &HashMap::new(),
                latest_text: Some("исходный + правка"),
                previous_text: None,
                last_edit: Some(at(20)),
            },
            at(20) + IDLE + Duration::from_secs(1),
        );
        assert_eq!(
            p,
            MilestonePlan::CreateNew {
                baseline: None,
                snapshot: Snapshot {
                    text: "новая мысль".to_owned().into(),
                    source: Source::Me,
                    at: at(20) + IDLE + Duration::from_secs(1),
                }
            }
        );
    }

    #[test]
    fn typing_past_cap_starts_new_version() {
        let v = version_file("v1.md", 20, Source::Me, "dev1");
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: "текст",
                disk_before: None,
            },
            &Recorded {
                versions: &[v],
                names: &HashMap::new(),
                latest_text: Some("текст пред"),
                previous_text: None,
                last_edit: Some(at(20) + CAP - Duration::from_secs(10)),
            },
            at(20) + CAP + Duration::from_secs(1),
        );
        assert_eq!(
            p,
            MilestonePlan::CreateNew {
                baseline: None,
                snapshot: Snapshot {
                    text: "текст".to_owned().into(),
                    source: Source::Me,
                    at: at(20) + CAP + Duration::from_secs(1),
                }
            }
        );
    }

    #[test]
    fn big_change_starts_new_version() {
        let v = version_file("v1.md", 20, Source::Me, "dev1");
        let big = "а".repeat(BIG_CHANGE_CHARS + 5);
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: &big,
                disk_before: None,
            },
            &Recorded {
                versions: &[v],
                names: &HashMap::new(),
                latest_text: Some("короткий"),
                previous_text: None,
                last_edit: Some(at(20)),
            },
            at(25),
        );
        assert_eq!(
            p,
            MilestonePlan::CreateNew {
                baseline: None,
                snapshot: Snapshot {
                    text: big.clone().into(),
                    source: Source::Me,
                    at: at(25),
                }
            }
        );
    }

    #[test]
    fn agent_edit_always_creates_new_version() {
        let v = version_file("v1.md", 20, Source::Me, "dev1");
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Agent,
                text: "правка агента",
                disk_before: None,
            },
            &Recorded {
                versions: &[v],
                names: &HashMap::new(),
                latest_text: Some("моё"),
                previous_text: None,
                last_edit: Some(at(20)),
            },
            at(25),
        );
        assert_eq!(
            p,
            MilestonePlan::CreateNew {
                baseline: None,
                snapshot: Snapshot {
                    text: "правка агента".to_owned().into(),
                    source: Source::Agent,
                    at: at(25),
                }
            }
        );
    }

    #[test]
    fn undo_to_previous_version_removes_draft() {
        let v1 = version_file("v1.md", 10, Source::Start, "dev1");
        let v2 = version_file("v2.md", 20, Source::Me, "dev1");
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: "исходный",
                disk_before: None,
            },
            &Recorded {
                versions: &[v1, v2],
                names: &HashMap::new(),
                latest_text: Some("исходный + черновик"),
                previous_text: Some("исходный"),
                last_edit: Some(at(20)),
            },
            at(22),
        );
        assert_eq!(
            p,
            MilestonePlan::RevertToPrevious {
                remove_id: "v2.md".to_owned()
            }
        );
    }

    #[test]
    fn reading_progress_is_quiet() {
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Reading,
                text: "позиция 10",
                disk_before: None,
            },
            &Recorded {
                versions: &[],
                names: &HashMap::new(),
                latest_text: None,
                previous_text: None,
                last_edit: None,
            },
            at(20),
        );
        assert_eq!(p, MilestonePlan::Quiet);
    }

    #[test]
    fn identical_text_skips() {
        let v = version_file("v1.md", 20, Source::Me, "dev1");
        let p = plan(
            "dev1",
            &Incoming {
                source: Source::Me,
                text: "то же",
                disk_before: None,
            },
            &Recorded {
                versions: &[v],
                names: &HashMap::new(),
                latest_text: Some("то же"),
                previous_text: None,
                last_edit: Some(at(20)),
            },
            at(25),
        );
        assert_eq!(p, MilestonePlan::Skip);
    }

    #[test]
    fn restore_and_revert_labels_roundtrip() {
        for source in [Source::Restore(1_790_000_000_123), Source::Revert(1_790_000_000_123)] {
            assert_eq!(Source::from_label(&source.file_label()), Some(source));
        }
        assert_eq!(Source::from_label("restore-"), None);
        assert_eq!(Source::from_label("me"), Some(Source::Me));
    }

    #[test]
    fn changed_chars_counts_unicode_characters() {
        assert_eq!(changed_chars("абв", "абв"), 0);
        assert_eq!(changed_chars("абв", "аXв"), 1);
        assert_eq!(changed_chars("аб", "ав"), 1);
        assert_eq!(changed_chars("", "ёжик"), 4);
    }
}
