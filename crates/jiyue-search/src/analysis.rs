//! Text analysis shared by indexing, query parsing and highlighting.
//!
//! Rules:
//! - Full-width ASCII forms (U+FF01..U+FF5E) and the ideographic space fold to half-width.
//! - A run of letters, digits and `_` is one lowercase token. Everything else separates tokens.
//! - An identifier-like run (containing `_`, a camelCase or acronym boundary, or a letter/digit
//!   transition) also yields its parts, stacked on its slots: `render_visible` gives the whole
//!   token plus `render` and `visible`; `renderVisible` gives `rendervisible` plus `render` and
//!   `visible`. The identifier takes one slot per part, so phrases line up with the parts.
//!   Query-side words yield only the whole token, but advance the slot counter the same way.
//! - Every CJK ideograph, kana, or hangul syllable takes one position slot. At index time it
//!   yields a unigram, and a bigram with the following character when the two are adjacent.
//!   The bigram is stacked on the slot of its first character.
//! - Query-side CJK runs use bigrams only (a single character uses its unigram), so a query
//!   bigram chain lines up with the stacked bigrams of the index.
//!
//! Offsets are byte offsets into the original text, so they stay valid after folding.

use std::collections::VecDeque;
use std::ops::Range;

use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

/// Name under which the analyzer is registered in the tantivy index.
pub(crate) const ANALYZER_NAME: &str = "jiyue";

/// One analyzed token. Offsets are byte offsets into the original text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyzedToken {
    pub text: String,
    pub start: usize,
    pub end: usize,
    /// Position slot. Tokens sharing a slot are stacked (a unigram and a bigram).
    pub slot: usize,
}

/// Folds full-width forms to their half-width equivalents.
pub(crate) fn fold(c: char) -> char {
    match c {
        '\u{3000}' => ' ',
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        _ => c,
    }
}

/// True for ideographs, kana, hangul and half-width katakana. Such characters form bigrams.
pub(crate) fn is_cjk(c: char) -> bool {
    if c == '\u{30FB}' {
        // Katakana middle dot is punctuation, not a letter.
        return false;
    }
    matches!(c as u32,
        0x3005 | 0x3007
        | 0x3040..=0x30FF
        | 0x31F0..=0x31FF
        | 0x3130..=0x318F
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x1100..=0x11FF
        | 0xAC00..=0xD7AF
        | 0xFF66..=0xFF9D
        | 0x20000..=0x2EBEF
        | 0x30000..=0x3134F)
}

fn is_word_char(c: char) -> bool {
    c == '_' || (c.is_alphanumeric() && !is_cjk(c))
}

/// Truncates `s` to at most `max_bytes` bytes, on a char boundary.
pub(crate) fn truncate_to(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A part of an identifier-like word. Offsets are byte offsets into the original text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Part {
    text: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
enum Unit {
    Word {
        text: String,
        start: usize,
        end: usize,
        /// Parts of an identifier-like word; empty for a plain word.
        parts: Vec<Part>,
    },
    Cjk {
        ch: char,
        start: usize,
        end: usize,
    },
}

/// Lazily splits text into words and single CJK characters.
struct Units<'a> {
    text: &'a str,
    pos: usize,
}

impl Iterator for Units<'_> {
    type Item = Unit;

    fn next(&mut self) -> Option<Unit> {
        loop {
            let raw = self.text[self.pos..].chars().next()?;
            let start = self.pos;
            self.pos += raw.len_utf8();
            let c = fold(raw);
            if is_cjk(c) {
                return Some(Unit::Cjk {
                    ch: c,
                    start,
                    end: self.pos,
                });
            }
            if is_word_char(c) {
                // Folded char with its byte span in the original text.
                let mut chars = vec![(c, start, self.pos)];
                let mut end = self.pos;
                while let Some(next) = self.text[end..].chars().next() {
                    let nc = fold(next);
                    if is_cjk(nc) || !is_word_char(nc) {
                        break;
                    }
                    chars.push((nc, end, end + next.len_utf8()));
                    end += next.len_utf8();
                }
                self.pos = end;
                let text = lowercase(&chars);
                let parts = word_parts(&chars);
                return Some(Unit::Word {
                    text,
                    start,
                    end,
                    parts,
                });
            }
            // Anything else is a separator.
        }
    }
}

/// Lowercased text of a run of folded chars.
fn lowercase(chars: &[(char, usize, usize)]) -> String {
    let mut out = String::with_capacity(chars.len());
    for (c, _, _) in chars {
        out.extend(c.to_lowercase());
    }
    out
}

/// True when a new identifier part starts between `prev` and `cur`: camelCase
/// (`renderVisible`), the end of an acronym (`HTTPServer`), or a letter/digit transition.
fn starts_part(prev: char, cur: char, next: Option<char>) -> bool {
    (prev.is_lowercase() && cur.is_uppercase())
        || (prev.is_uppercase() && cur.is_uppercase() && next.is_some_and(char::is_lowercase))
        || (prev.is_alphabetic() && cur.is_numeric())
        || (prev.is_numeric() && cur.is_alphabetic())
}

/// Index ranges (into `chars`) of the parts of a word. Underscores separate and are dropped.
fn identifier_segments(chars: &[(char, usize, usize)]) -> Vec<Range<usize>> {
    let mut segments = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..chars.len() {
        let cur = chars[i].0;
        if cur == '_' {
            if let Some(s) = start.take() {
                segments.push(s..i);
            }
            continue;
        }
        match start {
            None => start = Some(i),
            Some(s) => {
                let prev = chars[i - 1].0;
                let next = chars.get(i + 1).map(|c| c.0);
                if starts_part(prev, cur, next) {
                    segments.push(s..i);
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        segments.push(s..chars.len());
    }
    segments
}

/// Parts of an identifier-like word. Empty when the word needs no extra tokens: a plain word,
/// or a word whose only part is the whole word.
fn word_parts(chars: &[(char, usize, usize)]) -> Vec<Part> {
    let segments = identifier_segments(chars);
    if segments.is_empty() || (segments.len() == 1 && segments[0] == (0..chars.len())) {
        return Vec::new();
    }
    segments
        .into_iter()
        .map(|r| Part {
            text: lowercase(&chars[r.clone()]),
            start: chars[r.start].1,
            end: chars[r.end - 1].2,
        })
        .collect()
}

fn units(text: &str) -> Units<'_> {
    Units { text, pos: 0 }
}

#[derive(Debug, Clone)]
struct CjkPrev {
    ch: char,
    start: usize,
    end: usize,
    slot: usize,
}

/// Streaming index-time analysis: unigrams plus stacked bigrams. Yields tokens in
/// non-decreasing slot order.
pub(crate) struct IndexTokens<'a> {
    units: Units<'a>,
    slot: usize,
    prev: Option<CjkPrev>,
    pending: VecDeque<AnalyzedToken>,
}

/// Index-time tokens of `text`.
pub(crate) fn index_tokens(text: &str) -> IndexTokens<'_> {
    IndexTokens {
        units: units(text),
        slot: 0,
        prev: None,
        pending: VecDeque::new(),
    }
}

impl Iterator for IndexTokens<'_> {
    type Item = AnalyzedToken;

    fn next(&mut self) -> Option<AnalyzedToken> {
        if let Some(tok) = self.pending.pop_front() {
            return Some(tok);
        }
        match self.units.next()? {
            Unit::Word {
                text,
                start,
                end,
                parts,
            } => {
                self.prev = None;
                let slot = self.slot;
                // The whole word shares the slot of its first part; each further part takes
                // the next slot, so a phrase over the parts lines up.
                self.slot += parts.len().max(1);
                for (i, part) in parts.into_iter().enumerate() {
                    self.pending.push_back(AnalyzedToken {
                        text: part.text,
                        start: part.start,
                        end: part.end,
                        slot: slot + i,
                    });
                }
                Some(AnalyzedToken {
                    text,
                    start,
                    end,
                    slot,
                })
            }
            Unit::Cjk { ch, start, end } => {
                let slot = self.slot;
                self.slot += 1;
                let unigram = AnalyzedToken {
                    text: ch.to_string(),
                    start,
                    end,
                    slot,
                };
                let bigram = match self.prev.take() {
                    Some(prev) if prev.end == start => Some(AnalyzedToken {
                        text: format!("{}{}", prev.ch, ch),
                        start: prev.start,
                        end,
                        slot: prev.slot,
                    }),
                    _ => None,
                };
                self.prev = Some(CjkPrev {
                    ch,
                    start,
                    end,
                    slot,
                });
                match bigram {
                    Some(bi) => {
                        // Bigram first (its slot is the previous character's), then the unigram.
                        self.pending.push_back(unigram);
                        Some(bi)
                    }
                    None => Some(unigram),
                }
            }
        }
    }
}

/// Query-side analysis. A CJK run of two or more characters yields its overlapping bigrams;
/// a single character yields its unigram. Slots match the index-time slots.
pub(crate) fn query_tokens(text: &str) -> Vec<AnalyzedToken> {
    let units: Vec<Unit> = units(text).collect();
    let mut out = Vec::new();
    let mut slot = 0usize;
    let mut i = 0usize;
    while i < units.len() {
        match &units[i] {
            Unit::Word {
                text,
                start,
                end,
                parts,
            } => {
                // Only the whole word is queried; the slot advance mirrors the index.
                out.push(AnalyzedToken {
                    text: text.clone(),
                    start: *start,
                    end: *end,
                    slot,
                });
                slot += parts.len().max(1);
                i += 1;
            }
            Unit::Cjk { ch, start, end } => {
                let mut j = i + 1;
                while j < units.len() && cjk_adjacent(&units[j - 1], &units[j]) {
                    j += 1;
                }
                if j - i == 1 {
                    out.push(AnalyzedToken {
                        text: ch.to_string(),
                        start: *start,
                        end: *end,
                        slot,
                    });
                } else {
                    for k in i..j - 1 {
                        if let (
                            Unit::Cjk {
                                ch: a, start: s, ..
                            },
                            Unit::Cjk { ch: b, end: e, .. },
                        ) = (&units[k], &units[k + 1])
                        {
                            out.push(AnalyzedToken {
                                text: format!("{a}{b}"),
                                start: *s,
                                end: *e,
                                slot: slot + (k - i),
                            });
                        }
                    }
                }
                slot += j - i;
                i = j;
            }
        }
    }
    out
}

fn cjk_adjacent(prev: &Unit, next: &Unit) -> bool {
    match (prev, next) {
        (Unit::Cjk { end, .. }, Unit::Cjk { start, .. }) => end == start,
        _ => false,
    }
}

/// tantivy tokenizer that wraps [`index_tokens`].
#[derive(Clone, Default)]
pub(crate) struct JiyueTokenizer;

pub(crate) struct JiyueTokenStream<'a> {
    tokens: IndexTokens<'a>,
    token: Token,
}

impl Tokenizer for JiyueTokenizer {
    type TokenStream<'a> = JiyueTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        JiyueTokenStream {
            tokens: index_tokens(text),
            token: Token::default(),
        }
    }
}

impl TokenStream for JiyueTokenStream<'_> {
    fn advance(&mut self) -> bool {
        match self.tokens.next() {
            Some(tok) => {
                self.token.reset();
                self.token.offset_from = tok.start;
                self.token.offset_to = tok.end;
                self.token.position = tok.slot;
                self.token.text.push_str(&tok.text);
                true
            }
            None => false,
        }
    }

    fn token(&self) -> &Token {
        &self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<String> {
        index_tokens(s).map(|t| t.text).collect()
    }

    fn pairs(s: &str) -> Vec<(String, usize)> {
        index_tokens(s).map(|t| (t.text, t.slot)).collect()
    }

    fn query(s: &str) -> Vec<(String, usize)> {
        query_tokens(s)
            .into_iter()
            .map(|t| (t.text, t.slot))
            .collect()
    }

    fn p(text: &str, slot: usize) -> (String, usize) {
        (text.to_string(), slot)
    }

    #[test]
    fn latin_words_are_lowercased() {
        assert_eq!(texts("Hello WORLD"), vec!["hello", "world"]);
    }

    #[test]
    fn underscore_is_kept_inside_words() {
        // The whole identifier is kept, and its parts are stacked after it.
        assert_eq!(
            texts("foo_bar baz _x"),
            vec!["foo_bar", "foo", "bar", "baz", "_x", "x"]
        );
    }

    #[test]
    fn punctuation_splits_words() {
        assert_eq!(
            texts("async/await,rust.lang-kit"),
            vec!["async", "await", "rust", "lang", "kit"]
        );
    }

    #[test]
    fn digits_are_tokens() {
        assert_eq!(texts("2024-01-05"), vec!["2024", "01", "05"]);
    }

    #[test]
    fn accented_latin_is_one_word() {
        assert_eq!(texts("Café naïve"), vec!["café", "naïve"]);
    }

    #[test]
    fn mixed_chinese_and_english_index_tokens() {
        assert_eq!(
            pairs("Rust 异步编程 async/await"),
            vec![
                p("rust", 0),
                p("异", 1),
                p("异步", 1),
                p("步", 2),
                p("步编", 2),
                p("编", 3),
                p("编程", 3),
                p("程", 4),
                p("async", 5),
                p("await", 6),
            ]
        );
    }

    #[test]
    fn fullwidth_digits_are_normalised() {
        assert_eq!(pairs("２０２４年"), vec![p("2024", 0), p("年", 1)]);
    }

    #[test]
    fn fullwidth_latin_is_normalised_and_lowercased() {
        assert_eq!(texts("ＲＵＳＴ ＡＳＹＮＣ"), vec!["rust", "async"]);
    }

    #[test]
    fn fullwidth_punctuation_separates() {
        // U+FF0C is a full-width comma: it must not glue the CJK characters together.
        assert_eq!(pairs("异，步"), vec![p("异", 0), p("步", 1)]);
    }

    #[test]
    fn ideographic_space_separates_cjk_runs() {
        assert_eq!(pairs("异\u{3000}步"), vec![p("异", 0), p("步", 1)]);
    }

    #[test]
    fn ascii_space_breaks_cjk_bigrams() {
        assert_eq!(texts("异 步"), vec!["异", "步"]);
    }

    #[test]
    fn emoji_is_a_separator() {
        assert_eq!(texts("rust🚀async 🎉"), vec!["rust", "async"]);
    }

    #[test]
    fn emoji_only_text_yields_nothing() {
        assert!(texts("🚀🎉👍").is_empty());
    }

    #[test]
    fn empty_and_whitespace_text_yield_nothing() {
        assert!(texts("").is_empty());
        assert!(texts("   \n\t ").is_empty());
        assert!(query_tokens("").is_empty());
    }

    #[test]
    fn kana_and_hangul_form_bigrams() {
        assert_eq!(
            texts("こんにちは"),
            vec!["こ", "こん", "ん", "んに", "に", "にち", "ち", "ちは", "は"]
        );
        assert_eq!(texts("한국어"), vec!["한", "한국", "국", "국어", "어"]);
    }

    #[test]
    fn halfwidth_katakana_counts_as_cjk() {
        assert_eq!(texts("ｱｲ"), vec!["ｱ", "ｱｲ", "ｲ"]);
    }

    #[test]
    fn offsets_point_into_the_original_text() {
        let text = "Ａ异B";
        let toks: Vec<AnalyzedToken> = index_tokens(text).collect();
        assert_eq!(toks[0].text, "a");
        assert_eq!(&text[toks[0].start..toks[0].end], "Ａ");
        assert_eq!(&text[toks[1].start..toks[1].end], "异");
        assert_eq!(toks[2].text, "b");
        assert_eq!(&text[toks[2].start..toks[2].end], "B");
        assert_eq!(toks.len(), 3);
        for t in &toks {
            assert!(text.is_char_boundary(t.start));
            assert!(text.is_char_boundary(t.end));
        }
    }

    #[test]
    fn bigram_offsets_cover_both_characters() {
        let text = "x异步";
        let bigram = index_tokens(text)
            .find(|t| t.text == "异步")
            .expect("bigram");
        assert_eq!(&text[bigram.start..bigram.end], "异步");
    }

    #[test]
    fn index_tokens_are_in_non_decreasing_slot_order() {
        let slots: Vec<usize> = index_tokens("一二三 abc 四五 六").map(|t| t.slot).collect();
        assert!(slots.windows(2).all(|w| w[0] <= w[1]), "{slots:?}");
    }

    #[test]
    fn query_single_cjk_char_is_unigram() {
        assert_eq!(query("步"), vec![p("步", 0)]);
    }

    #[test]
    fn query_cjk_run_is_bigram_chain() {
        assert_eq!(
            query("异步编程"),
            vec![p("异步", 0), p("步编", 1), p("编程", 2)]
        );
    }

    #[test]
    fn query_mixed_latin_and_cjk() {
        assert_eq!(query("rust异步"), vec![p("rust", 0), p("异步", 1)]);
    }

    #[test]
    fn query_slots_agree_with_index_slots() {
        // Query slots are relative; the phrase matches when every query token sits at the same
        // offset from the first one in the document.
        let doc: Vec<(String, usize)> = pairs("学习Rust异步编程");
        let base = doc
            .iter()
            .find(|(t, _)| t == "异步")
            .map(|(_, s)| *s)
            .expect("bigram in doc");
        for (text, slot) in query("异步编程") {
            assert!(
                doc.contains(&(text.clone(), base + slot)),
                "{text}@{slot} not in {doc:?}"
            );
        }
    }

    #[test]
    fn cjk_and_symbols_only_query() {
        assert_eq!(query("！！"), Vec::<(String, usize)>::new());
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "异".repeat(10); // 30 bytes
        let t = truncate_to(&s, 10);
        assert!(t.len() <= 10);
        assert_eq!(t.chars().count(), 3);
        assert_eq!(truncate_to("abc", 10), "abc");
    }

    #[test]
    fn truncate_to_one_mebibyte_is_valid_utf8() {
        let s = "中".repeat(1 << 20);
        let t = truncate_to(&s, 1 << 20);
        assert!(t.len() <= 1 << 20);
        assert!(t.len() > (1 << 20) - 3);
        assert_eq!(t.len() % 3, 0);
    }

    #[test]
    fn cjk_classification() {
        for c in ['中', 'あ', 'ア', '한', 'ｱ', '々'] {
            assert!(is_cjk(c), "{c} should be CJK");
        }
        for c in ['a', 'é', '１', '。', '・', '🚀'] {
            assert!(!is_cjk(c), "{c} should not be CJK");
        }
    }

    #[test]
    fn fold_maps_fullwidth_ascii() {
        assert_eq!(fold('！'), '!');
        assert_eq!(fold('Ａ'), 'A');
        assert_eq!(fold('\u{3000}'), ' ');
        assert_eq!(fold('中'), '中');
    }

    #[test]
    fn tantivy_tokenizer_emits_same_tokens() {
        let mut tokenizer = JiyueTokenizer;
        let mut stream = tokenizer.token_stream("Rust 异步");
        let mut got = Vec::new();
        while stream.advance() {
            got.push((stream.token().text.clone(), stream.token().position));
        }
        let expected: Vec<(String, usize)> = pairs("Rust 异步");
        assert_eq!(got, expected);
    }

    #[test]
    fn snake_case_identifier_adds_parts_on_its_slot() {
        assert_eq!(
            pairs("pub fn render_visible"),
            vec![
                p("pub", 0),
                p("fn", 1),
                p("render_visible", 2),
                p("render", 2),
                p("visible", 3),
            ]
        );
    }

    #[test]
    fn camel_case_identifier_is_split() {
        assert_eq!(
            pairs("renderVisible"),
            vec![p("rendervisible", 0), p("render", 0), p("visible", 1)]
        );
    }

    #[test]
    fn acronym_and_digit_boundaries_split() {
        assert_eq!(
            texts("HTTPServer utf8Parser"),
            vec![
                "httpserver",
                "http",
                "server",
                "utf8parser",
                "utf",
                "8",
                "parser"
            ]
        );
    }

    #[test]
    fn plain_words_and_numbers_get_no_parts() {
        assert_eq!(
            texts("rust 2024-01 Café"),
            vec!["rust", "2024", "01", "café"]
        );
        // Letter/digit transition inside one word: whole token plus parts.
        assert_eq!(texts("v1.2"), vec!["v1", "v", "1", "2"]);
    }

    #[test]
    fn identifier_part_offsets_point_into_the_original_text() {
        let text = "callRenderVisible(x_y)";
        let toks: Vec<AnalyzedToken> = index_tokens(text).collect();
        for t in &toks {
            assert!(text.is_char_boundary(t.start) && text.is_char_boundary(t.end));
        }
        let slice = |t: &AnalyzedToken| &text[t.start..t.end];
        let render = toks.iter().find(|t| t.text == "render").expect("render");
        assert_eq!(slice(render), "Render");
        let visible = toks.iter().find(|t| t.text == "visible").expect("visible");
        assert_eq!(slice(visible), "Visible");
        let whole = toks.iter().find(|t| t.text == "callrendervisible");
        assert!(whole.is_some());
        let y = toks.iter().find(|t| t.text == "y").expect("y");
        assert_eq!(slice(y), "y");
    }

    #[test]
    fn phrase_over_identifier_parts_lines_up_with_later_tokens() {
        // Query side: the identifier advances the slot by its part count.
        assert_eq!(
            query("render_visible foo"),
            vec![p("render_visible", 0), p("foo", 2)]
        );
        assert_eq!(
            query("render visible"),
            vec![p("render", 0), p("visible", 1)]
        );
        let doc = pairs("render_visible foo");
        assert!(doc.contains(&p("foo", 2)));
    }

    #[test]
    fn mixed_cjk_and_identifier_keep_bigrams_and_slots() {
        // The CJK run after an identifier starts a fresh bigram chain.
        assert_eq!(
            pairs("学习render_visible函数"),
            vec![
                p("学", 0),
                p("学习", 0),
                p("习", 1),
                p("render_visible", 2),
                p("render", 2),
                p("visible", 3),
                p("函", 4),
                p("函数", 4),
                p("数", 5),
            ]
        );
        assert_eq!(query("visible函数"), vec![p("visible", 0), p("函数", 1)]);
    }
}
