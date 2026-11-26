use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, RemoveLongFilter, TextAnalyzer, Tokenizer};
use super::{filters::{ExclusionFilter, SanitizingTokenizer}, morphology::StemFilter};

pub const TOKENIZER_NAME: &str = "aquilum_text";
pub const ANALYZER_VERSION: u32 = 12;
pub fn indexing_tokenizer() -> TextAnalyzer { word_analyzer(SanitizingTokenizer::default()) }

pub(super) fn word_analyzer(tokenizer: impl Tokenizer) -> TextAnalyzer {
    TextAnalyzer::builder(tokenizer)
        .filter(RemoveLongFilter::limit(120))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(StemFilter)
        .filter(ExclusionFilter)
        .build()
}
