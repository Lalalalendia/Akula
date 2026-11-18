//! LANG-02/05: first-party, in-memory practice projection of selected study groups.
//!
//! Inputs are one validated atomic StudyState and complete, CURRENT snapshots
//! of notes that the trusted native host has already authorized. Persisted
//! table identity is not a permission to open a vault or reuse stale text.
//! No card text, answers, progress, or other source content is persisted here.

use std::collections::{BTreeMap, BTreeSet};

use super::activation::{self, ActivationError, LiveNote};
use super::collections::Limits;
use super::identity::Source;
use super::state::{self, StateError, StudyState};

pub const MAX_PRACTICE_TABLES: usize = 256;
pub const MAX_PRACTICE_CARDS: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PracticeError {
    InvalidState(StateError),
    Activation(ActivationError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PracticeCard {
    /// Stable source table identity, NOT authority to read its vault.
    pub source: Source,
    /// Ephemeral row in the currently authorized table, not an SRS row ID.
    pub source_row: usize,
    pub question: String,
    pub accepted_answers: Vec<String>,
    pub context: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PracticeDeck {
    pub cards: Vec<PracticeCard>,
    /// IDs only; never disclose content from unavailable/held sources.
    pub unavailable: Vec<Source>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceQuestion {
    pub source: Source,
    pub source_row: usize,
    pub question: String,
    pub context: Option<String>,
    /// All displayed options come from currently authorized source cells.
    pub options: Vec<String>,
    pub correct_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPair {
    pub source: Source,
    pub source_row: usize,
    pub question: String,
    pub answer: String,
}

/// Deduplicate overlap by full (vault,note,table) ID, but keep distinct rows
/// and different senses of the same term separate. Never use stored row offsets
/// to load text; live table content is verified by complete table fingerprints.
///
/// The caller must check the current file/vault read grant AND document revision
/// before supplying LiveNote, and recheck before acting on returned source rows.
pub fn build_deck(
    state: &StudyState,
    collection_ids: &[String],
    authorized_notes: &[LiveNote<'_>],
    limits: Limits,
) -> Result<PracticeDeck, PracticeError> {
    state::validate(state).map_err(PracticeError::InvalidState)?;
    let bounded = Limits {
        max_tables: limits.max_tables.min(MAX_PRACTICE_TABLES),
        max_rows_per_table: limits.max_rows_per_table.min(MAX_PRACTICE_CARDS),
    };
    let verified = activation::assemble_verified(
        &state.registry, &state.anchors, collection_ids, authorized_notes, bounded,
    ).map_err(PracticeError::Activation)?;
    let mut cards = Vec::new();
    let mut truncated = false;
    for table in verified.tables {
        truncated |= table.preview.truncated;
        for item in table.preview.items {
            if cards.len() == MAX_PRACTICE_CARDS {
                truncated = true;
                break;
            }
            cards.push(PracticeCard {
                source: table.source.clone(),
                source_row: item.source_row,
                question: item.question,
                accepted_answers: item.accepted_answers,
                context: item.context,
            });
        }
    }
    Ok(PracticeDeck { cards, unavailable: verified.unavailable, truncated })
}

fn normalized(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Assemble one "choose from 2–4" exercise without invented distractors.
/// If fewer than 3 safe alternatives exist, offer fewer options (at least 2).
/// Every known accepted translation for the SAME prompt, even from a different
/// row or collection, is excluded from wrong choices. This avoids marking
/// another known meaning of "run" as an incorrect answer for "run".
///
/// This is a display question, not a persistent grade or learning-history ID.
pub fn multiple_choice(
    deck: &PracticeDeck,
    card_index: usize,
    requested_options: usize,
) -> Option<ChoiceQuestion> {
    if !(2..=4).contains(&requested_options) {
        return None;
    }
    let target = deck.cards.get(card_index)?;
    let correct = target.accepted_answers.first()?.clone();
    let question = normalized(&target.question);
    let accepted: BTreeSet<String> = deck.cards.iter()
        .filter(|card| normalized(&card.question) == question)
        .flat_map(|card| card.accepted_answers.iter().map(|answer| normalized(answer)))
        .collect();
    let mut unique_wrong = BTreeSet::new();
    let mut options = Vec::new();
    for card in &deck.cards {
        if normalized(&card.question) == question {
            continue;
        }
        for answer in &card.accepted_answers {
            let folded = normalized(answer);
            if !folded.is_empty()
                && !accepted.contains(&folded)
                && unique_wrong.insert(folded)
            {
                options.push(answer.clone());
                if options.len() == requested_options - 1 {
                    break;
                }
            }
        }
        if options.len() == requested_options - 1 {
            break;
        }
    }
    if options.is_empty() {
        return None;
    }
    let correct_index = card_index % (options.len() + 1);
    options.insert(correct_index, correct);
    Some(ChoiceQuestion {
        source: target.source.clone(),
        source_row: target.source_row,
        question: target.question.clone(),
        context: target.context.clone(),
        options,
        correct_index,
    })
}

/// Conservative matching pairs. Ambiguous question/answer strings and
/// multi-answer entries are excluded, not silently arbitrated by the game.
/// Not enough safe pairs? Return an empty/smaller exercise; flashcards remain.
pub fn matching_pairs(deck: &PracticeDeck, max_pairs: usize) -> Vec<MatchPair> {
    let mut question_counts = BTreeMap::<String, usize>::new();
    let mut answer_counts = BTreeMap::<String, usize>::new();
    for card in &deck.cards {
        *question_counts.entry(normalized(&card.question)).or_default() += 1;
        for answer in &card.accepted_answers {
            *answer_counts.entry(normalized(answer)).or_default() += 1;
        }
    }
    deck.cards.iter().filter_map(|card| {
        let [answer] = card.accepted_answers.as_slice() else { return None };
        if question_counts.get(&normalized(&card.question)) != Some(&1)
            || answer_counts.get(&normalized(answer)) != Some(&1)
        {
            return None;
        }
        Some(MatchPair {
            source: card.source.clone(),
            source_row: card.source_row,
            question: card.question.clone(),
            answer: answer.clone(),
        })
    }).take(max_pairs.min(32)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ColumnMapping;
    use super::super::state::bind_table;
    use super::super::table_anchors::NoteKey;

    fn note(id: &str) -> NoteKey {
        NoteKey { vault_id: "vault-A".into(), note_id: id.into() }
    }
    fn table(term: &str, answer: &str) -> Vec<Vec<String>> {
        vec![
            vec!["Question".into(), "Answer".into()],
            vec![term.into(), answer.into()],
        ]
    }
    fn mapping() -> ColumnMapping {
        ColumnMapping { question: 0, answers: vec![1], context: None }
    }
    fn limits() -> Limits {
        Limits { max_tables: 100, max_rows_per_table: 30 }
    }
    fn example() -> (StudyState, [NoteKey; 3], [Vec<Vec<Vec<String>>>; 3]) {
        let notes = [note("one"), note("two"), note("three")];
        let rows = [
            vec![table("eat", "есть"), table("ate", "ел")],
            vec![table("read", "читать")],
            vec![table("write", "писать")],
        ];
        let mut state = StudyState::default();
        bind_table(&mut state, &notes[0], &rows[0], 0, mapping(), "en", "English", "table-a").unwrap();
        bind_table(&mut state, &notes[0], &rows[0], 1, mapping(), "en", "English", "table-b").unwrap();
        bind_table(&mut state, &notes[0], &rows[0], 1, mapping(), "verbs", "Verbs", "ignored").unwrap();
        bind_table(&mut state, &notes[1], &rows[1], 0, mapping(), "en", "English", "table-c").unwrap();
        bind_table(&mut state, &notes[2], &rows[2], 0, mapping(), "verbs", "Verbs", "table-d").unwrap();
        (state, notes, rows)
    }

    #[test]
    fn one_atomic_state_unites_four_tables_from_three_notes_without_overlap() {
        let (state, notes, rows) = example();
        let live = [
            LiveNote { note: &notes[0], tables: &rows[0] },
            LiveNote { note: &notes[1], tables: &rows[1] },
            LiveNote { note: &notes[2], tables: &rows[2] },
        ];
        let result = build_deck(&state, &["en".into(), "verbs".into(), "en".into()], &live, limits()).unwrap();
        assert_eq!(result.cards.len(), 4);
        assert!(!result.truncated);
        assert!(result.unavailable.is_empty());
        assert_eq!(result.cards.iter().map(|c| c.question.as_str()).collect::<BTreeSet<_>>(),
            ["eat", "ate", "read", "write"].into_iter().collect());
        assert_eq!(result.cards.iter().filter(|c| c.question == "ate").count(), 1);
        assert_eq!(state.registry.tables.len(), 4);
    }

    #[test]
    fn persisted_atomic_store_builds_deduplicated_deck_after_reopen() {
        use super::super::state::StudyStore;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("language-study.sqlite");
        let (_, notes, rows) = example();
        let mut store = StudyStore::open(&path).unwrap();
        let mut revision = 0;
        for (note_index, table_index, group_id, group_name, candidate) in [
            (0, 0, "en", "English", "table-a"),
            (0, 1, "en", "English", "table-b"),
            (0, 1, "verbs", "Verbs", "unused-id"),
            (1, 0, "en", "English", "table-c"),
            (2, 0, "verbs", "Verbs", "table-d"),
        ] {
            revision = store.bind(
                revision, &notes[note_index], &rows[note_index], table_index,
                mapping(), group_id, group_name, candidate,
            ).unwrap().revision;
        }
        assert_eq!(revision, 5);
        drop(store);
        let stored = StudyStore::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.revision, 5);
        assert_eq!(stored.state.registry.tables.len(), 4);
        let live = [
            LiveNote { note: &notes[0], tables: &rows[0] },
            LiveNote { note: &notes[1], tables: &rows[1] },
            LiveNote { note: &notes[2], tables: &rows[2] },
        ];
        let deck = build_deck(
            &stored.state, &["en".into(), "verbs".into()], &live, limits(),
        ).unwrap();
        assert_eq!(deck.cards.len(), 4);
        assert!(deck.unavailable.is_empty());
        assert!(multiple_choice(&deck, 0, 4).is_some());
        assert_eq!(matching_pairs(&deck, 4).len(), 4);
        // Persisted sidecar is configuration and opaque IDs, not vocabulary.
        let sidecar = stored.state.registry.collections;
        assert_eq!(sidecar[0].members.len(), 3);
        assert_eq!(sidecar[1].members.len(), 2);
    }

    #[test]
    fn absent_or_edited_notes_cannot_reveal_old_content_or_steal_identity() {
        let (state, notes, rows) = example();
        let changed = vec![table("eat", "кушать"), rows[0][1].clone()];
        let live = [
            LiveNote { note: &notes[0], tables: &changed },
            LiveNote { note: &notes[2], tables: &rows[2] },
        ];
        let result = build_deck(&state, &["en".into(), "verbs".into()], &live, limits()).unwrap();
        assert_eq!(result.cards.iter().map(|c| c.question.as_str()).collect::<Vec<_>>(),
            ["ate", "write"]);
        assert_eq!(result.unavailable.len(), 2);
        assert!(!result.cards.iter().any(|c| c.question == "eat" || c.question == "read"));
    }

    fn deck(cards: Vec<(&str, &[&str])>) -> PracticeDeck {
        let cards = cards.into_iter().enumerate().map(|(i, (q, answers))| PracticeCard {
            source: Source {
                vault_id: "vault-A".into(), note_id: "n".into(), table_id: i.to_string(),
            },
            source_row: i + 1,
            question: q.into(),
            accepted_answers: answers.iter().map(|a| (*a).into()).collect(),
            context: None,
        }).collect();
        PracticeDeck { cards, unavailable: vec![], truncated: false }
    }

    #[test]
    fn choice_never_marks_another_known_sense_of_run_wrong() {
        let data = deck(vec![
            ("run", &["бегать"]), ("run", &["работать"]), ("run", &["управлять"]),
            ("read", &["читать"]), ("sleep", &["спать"]), ("write", &["писать"]),
        ]);
        let q = multiple_choice(&data, 0, 4).unwrap();
        assert_eq!(q.options.len(), 4);
        assert_eq!(q.options[q.correct_index], "бегать");
        assert!(!q.options.iter().any(|s| s == "работать" || s == "управлять"));
        assert_eq!(q.options.iter().collect::<BTreeSet<_>>().len(), 4);
    }

    #[test]
    fn options_are_reduced_or_refused_without_inventing_a_distractor() {
        let small = deck(vec![("fast", &["быстрый", "скорый"]), ("quick", &["скорый"]),
                              ("slow", &["медленный"])]);
        let two = multiple_choice(&small, 0, 4).unwrap();
        assert_eq!(two.options.len(), 2);
        assert_eq!(two.options[two.correct_index], "быстрый");
        assert!(!two.options.contains(&"скорый".to_owned()));
        assert_eq!(multiple_choice(&small, 0, 5), None);
        let one = deck(vec![("fast", &["быстрый"]), ("FAST", &["скорый"])]);
        assert_eq!(multiple_choice(&one, 0, 4), None);
    }

    #[test]
    fn matching_skips_duplicate_prompts_answers_and_multi_answer_rows() {
        let data = deck(vec![
            ("run", &["бегать"]), ("run", &["управлять"]),
            ("cat", &["кот"]), ("kitten", &["кот"]),
            ("fast", &["быстрый", "скорый"]),
            ("read", &["читать"]), ("write", &["писать"]),
        ]);
        let pairs = matching_pairs(&data, 20);
        assert_eq!(pairs.iter().map(|p| p.question.as_str()).collect::<Vec<_>>(), ["read", "write"]);
        assert_eq!(matching_pairs(&data, 1).len(), 1);
        assert!(matching_pairs(&data, 0).is_empty());
    }

    #[test]
    fn invalid_stored_mapping_fails_before_projecting_any_text() {
        let (mut state, notes, rows) = example();
        state.registry.tables[0].source.table_id = "ghost".into();
        let result = build_deck(&state, &["en".into()], &[LiveNote {
            note: &notes[0], tables: &rows[0],
        }], limits());
        assert_eq!(result, Err(PracticeError::InvalidState(
            StateError::InvalidCollections(super::super::collections::CollectionError::MissingTableMapping)
        )));
        // If the membership is also forged to match, the missing anchor is
        // still caught before any source text can be projected.
        let group = state.registry.collections.iter_mut()
            .find(|g| g.id == "en").unwrap();
        group.members.iter_mut().find(|s| s.table_id == "table-a")
            .unwrap().table_id = "ghost".into();
        let result = build_deck(&state, &["en".into()], &[LiveNote {
            note: &notes[0], tables: &rows[0],
        }], limits());
        assert_eq!(result, Err(PracticeError::InvalidState(StateError::OrphanSource)));
    }

    #[test]
    fn row_cap_never_promotes_ephemeral_row_positions_to_persistent_ids() {
        let (state, notes, rows) = example();
        let all = [LiveNote { note: &notes[0], tables: &rows[0] }];
        let output = build_deck(&state, &["en".into()], &all,
            Limits { max_tables: 10, max_rows_per_table: 0 }).unwrap();
        assert!(output.cards.is_empty());
        assert!(output.truncated);
        assert_eq!(output.unavailable.len(), 1);
    }
}
