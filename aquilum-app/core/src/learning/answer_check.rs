//! LANG-04: conservative, read-only validation of manually entered answers.
//!
//! The trusted host supplies a CURRENT, authorized practice deck and must
//! revalidate it before displaying or grading an answer. No source cells,
//! history, grades, permissions or Markdown are written by this module.

use std::collections::BTreeSet;
use unicode_normalization::UnicodeNormalization;

use super::practice::{PracticeCard, PracticeDeck};

pub const MAX_TYPED_ANSWER_BYTES: usize = 4096;

/// Only Correct and Incorrect are gradeable. The other outcomes must not
/// advance spaced-repetition state or count as failed attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedVerdict {
    Correct,
    Incorrect,
    Empty,
    TooLong,
    Unavailable,
    NeedsContext,
    OtherKnownSense,
}

/// Compare canonical Unicode forms, disregarding extra whitespace and case.
/// This is not stemming, translation, or automatic synonym inference.
fn normalized(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.nfc().collect::<String>().to_lowercase().nfc().collect()
}

fn accepted(card: &PracticeCard) -> BTreeSet<String> {
    card.accepted_answers.iter().map(|value| normalized(value))
        .filter(|value| !value.is_empty()).collect()
}

/// Conservative pure projection for a typed-answer game. Caller must validate
/// the same source revisions and permissions immediately before using result.
/// The row offset is ephemeral and is never a review-history identity.
pub fn check_typed_answer(
    deck: &PracticeDeck,
    card_index: usize,
    submitted: &str,
) -> TypedVerdict {
    if submitted.len() > MAX_TYPED_ANSWER_BYTES {
        return TypedVerdict::TooLong;
    }
    let response = normalized(submitted);
    if response.is_empty() {
        return TypedVerdict::Empty;
    }
    let Some(target) = deck.cards.get(card_index) else {
        return TypedVerdict::Unavailable;
    };
    let answers = accepted(target);
    if answers.is_empty() {
        return TypedVerdict::Unavailable;
    }

    let question = normalized(&target.question);
    let target_context = target.context.as_deref().map(normalized)
        .filter(|context| !context.is_empty());
    let mut other_sense = false;
    for (index, card) in deck.cards.iter().enumerate() {
        if index == card_index || normalized(&card.question) != question {
            continue;
        }
        let other_answers = accepted(card);
        if other_answers == answers {
            continue;
        }
        let other_context = card.context.as_deref().map(normalized)
            .filter(|context| !context.is_empty());
        // Identical prompts with different accepted answers need distinct
        // contextual clues. An unqualified prompt cannot be fairly graded.
        if target_context.is_none() || other_context.is_none()
            || target_context == other_context
        {
            return TypedVerdict::NeedsContext;
        }
        other_sense |= other_answers.contains(&response);
    }

    if answers.contains(&response) {
        TypedVerdict::Correct
    } else if other_sense {
        // A correct translation of another known sense is not an invented
        // synonym, but should not be marked wrong without an explicit review.
        TypedVerdict::OtherKnownSense
    } else {
        TypedVerdict::Incorrect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::identity::Source;

    fn card(q: &str, answer: &[&str], context: Option<&str>, index: usize) -> PracticeCard {
        PracticeCard {
            source: Source {
                vault_id: "v".into(), note_id: "n".into(),
                table_id: format!("t-{index}"),
            },
            source_row: index + 1,
            question: q.into(),
            accepted_answers: answer.iter().map(|s| (*s).into()).collect(),
            context: context.map(str::to_owned),
        }
    }

    fn deck(cards: Vec<PracticeCard>) -> PracticeDeck {
        PracticeDeck { cards, unavailable: Vec::new(), truncated: false }
    }

    #[test]
    fn explicit_alternatives_case_whitespace_and_nfc_are_accepted() {
        let set = deck(vec![card(
            "fast", &["быстрый", "скорый", "café au lait"], None, 0,
        )]);
        assert_eq!(check_typed_answer(&set, 0, " СКОРЫЙ "), TypedVerdict::Correct);
        assert_eq!(check_typed_answer(&set, 0, "  cafe\u{301}  au  lait "),
            TypedVerdict::Correct);
        assert_eq!(check_typed_answer(&set, 0, "медленный"), TypedVerdict::Incorrect);
    }

    #[test]
    fn never_split_a_comma_separated_answer_or_guess_synonyms() {
        let set = deck(vec![card("run", &["бежать, бегать"], Some("fitness"), 0)]);
        assert_eq!(check_typed_answer(&set, 0, "бежать"), TypedVerdict::Incorrect);
        assert_eq!(check_typed_answer(&set, 0, "бежать, бегать"), TypedVerdict::Correct);
    }

    #[test]
    fn distinct_run_senses_require_context_to_grade() {
        let no_context = deck(vec![
            card("run", &["бегать"], None, 0),
            card("RUN", &["работать"], Some("program"), 1),
            card("run", &["управлять"], Some("company"), 2),
        ]);
        assert_eq!(check_typed_answer(&no_context, 0, "бегать"),
            TypedVerdict::NeedsContext);
        assert_eq!(check_typed_answer(&no_context, 0, "работать"),
            TypedVerdict::NeedsContext);
        let explained = deck(vec![
            card("run", &["бегать"], Some("every morning"), 0),
            card("run", &["работать"], Some("the program runs"), 1),
            card("run", &["управлять"], Some("a company"), 2),
        ]);
        assert_eq!(check_typed_answer(&explained, 0, "бегать"), TypedVerdict::Correct);
        assert_eq!(check_typed_answer(&explained, 0, "работать"),
            TypedVerdict::OtherKnownSense);
        assert_eq!(check_typed_answer(&explained, 0, "неизвестно"),
            TypedVerdict::Incorrect);
    }

    #[test]
    fn duplicate_prompt_with_same_context_but_different_senses_is_held() {
        let set = deck(vec![
            card("light", &["свет"], Some("example"), 0),
            card("light", &["лёгкий"], Some("EXAMPLE"), 1),
        ]);
        assert_eq!(check_typed_answer(&set, 0, "свет"), TypedVerdict::NeedsContext);
    }

    #[test]
    fn identical_sources_in_overlapping_collections_do_not_create_false_ambiguity() {
        let set = deck(vec![
            card("fast", &["быстрый"], None, 0),
            card("FAST", &["быстрый"], None, 1),
        ]);
        assert_eq!(check_typed_answer(&set, 0, "БЫСТРЫЙ"), TypedVerdict::Correct);
    }

    #[test]
    fn empty_unknown_and_oversized_responses_are_never_graded() {
        let set = deck(vec![card("eat", &["есть"], None, 0)]);
        assert_eq!(check_typed_answer(&set, 0, " \u{2003}\t "),
            TypedVerdict::Empty);
        assert_eq!(check_typed_answer(&set, 1, "есть"),
            TypedVerdict::Unavailable);
        assert_eq!(check_typed_answer(&set, 0, &"a".repeat(MAX_TYPED_ANSWER_BYTES + 1)),
            TypedVerdict::TooLong);
        assert_eq!(check_typed_answer(&deck(vec![card("x", &[], None, 0)]), 0, "a"),
            TypedVerdict::Unavailable);
    }

    #[test]
    fn changed_authorized_deck_changes_answer_without_persistence() {
        let original = deck(vec![card("eat", &["есть"], None, 0)]);
        let revised = deck(vec![card("eat", &["кушать"], None, 0)]);
        assert_eq!(check_typed_answer(&original, 0, "есть"), TypedVerdict::Correct);
        assert_eq!(check_typed_answer(&revised, 0, "есть"), TypedVerdict::Incorrect);
        assert_eq!(check_typed_answer(&revised, 0, "кушать"), TypedVerdict::Correct);
    }
}
