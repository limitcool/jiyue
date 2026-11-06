//! Finds math spans in Markdown text: `$...$`, `$$...$$`, `\(...\)`, `\[...\]`.
//!
//! Rules (close to Obsidian):
//! - Code spans (`` `x` ``) and fenced blocks (``` or ~~~) are skipped.
//! - `\$`, `\\` and other escaped characters never open or close math.
//! - `$` followed by whitespace does not open inline math.
//! - Inline `$` closes only if the closing `$` is not preceded by whitespace and
//!   not followed by a digit, so `costs $5 and $10` stays plain text.
//! - Inline `$` followed by a digit does not open (conservative currency guard;
//!   `$2^n$` therefore stays plain text: a known trade-off).
//! - Inline math stays on one line; display math may span lines within a
//!   paragraph (no blank line inside).
//!
//! Complexity: a failed closer search is remembered per delimiter kind, so a
//! document full of unmatched delimiters is scanned in roughly linear time.

use std::ops::Range;

/// One math span found in the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathSpan {
    /// Byte range of the whole span, delimiters included.
    pub range: Range<usize>,
    /// Trimmed inner LaTeX source.
    pub latex: String,
    /// `true` for `$$...$$` and `\[...\]`.
    pub display: bool,
}

/// Remembered failed search: a search for a closer starting at `from` and bounded
/// by `limit` found nothing. Any later search with the same `limit` and a start
/// at or after `from` cannot succeed either.
#[derive(Clone, Copy)]
struct Failed {
    from: usize,
    limit: usize,
}

#[derive(Default)]
struct Memo {
    para_end: Option<usize>,
    dollar_dollar: Option<Failed>,
    backslash: Option<Failed>,
    inline: Option<Failed>,
    line_end: Option<usize>,
}

/// End of the line containing `from`, cached. Scans are monotone, so a cached
/// end at or after `from` is still the first newline at or after `from`.
fn cached_line_end(b: &[u8], from: usize, memo: &mut Memo) -> usize {
    if let Some(end) = memo.line_end
        && end >= from
    {
        return end;
    }
    let end = line_end(b, from);
    memo.line_end = Some(end);
    end
}

fn known_failure(memo: Option<Failed>, from: usize, limit: usize) -> bool {
    memo.is_some_and(|f| f.limit == limit && from >= f.from)
}

/// Locate all math spans in `text`, in order, non-overlapping.
pub fn find_math_spans(text: &str) -> Vec<MathSpan> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut memo = Memo::default();
    let mut i = 0;
    while i < n {
        // Fenced code block at the start of a line.
        if (i == 0 || b[i - 1] == b'\n')
            && let Some(next) = fenced_block_end(b, i)
        {
            i = next;
            continue;
        }
        match b[i] {
            b'\\' => {
                let next = b.get(i + 1).copied();
                if matches!(next, Some(b'(') | Some(b'[')) {
                    let display = next == Some(b'[');
                    let limit = para_end(b, i, &mut memo);
                    let found = if known_failure(memo.backslash, i + 2, limit) {
                        None
                    } else {
                        let r = find_close_backslash(b, i + 2, limit, display);
                        if r.is_none() {
                            memo.backslash = Some(Failed { from: i + 2, limit });
                        }
                        r
                    };
                    match found {
                        Some(j) if !text[i + 2..j].trim().is_empty() => {
                            out.push(MathSpan {
                                range: i..j + 2,
                                latex: text[i + 2..j].trim().to_string(),
                                display,
                            });
                            i = j + 2;
                        }
                        _ => i += 2,
                    }
                } else {
                    // Skip the escaped character (covers \$ and \\).
                    i += if next.is_some() { 2 } else { 1 };
                }
            }
            b'`' => {
                let run = run_len(b, i, b'`');
                match find_backtick_run(b, i + run, run) {
                    Some(end) => i = end,
                    None => i += run,
                }
            }
            b'$' if b.get(i + 1) == Some(&b'$') => {
                let limit = para_end(b, i, &mut memo);
                let found = if known_failure(memo.dollar_dollar, i + 2, limit) {
                    None
                } else {
                    let r = find_close_dollar_dollar(b, i + 2, limit);
                    if r.is_none() {
                        memo.dollar_dollar = Some(Failed { from: i + 2, limit });
                    }
                    r
                };
                match found {
                    Some(j) if !text[i + 2..j].trim().is_empty() => {
                        out.push(MathSpan {
                            range: i..j + 2,
                            latex: text[i + 2..j].trim().to_string(),
                            display: true,
                        });
                        i = j + 2;
                    }
                    _ => i += 2,
                }
            }
            b'$' => {
                let limit = cached_line_end(b, i, &mut memo);
                let found = if known_failure(memo.inline, i + 1, limit) {
                    None
                } else {
                    let r = try_inline_dollar(b, i, limit);
                    if r.is_none() {
                        memo.inline = Some(Failed { from: i + 1, limit });
                    }
                    r
                };
                match found {
                    Some(j) => {
                        out.push(MathSpan {
                            range: i..j + 1,
                            latex: text[i + 1..j].trim().to_string(),
                            display: false,
                        });
                        i = j + 1;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Position of the first byte of the first blank line at or after `from`, or the
/// text length. Cached so that a long document is scanned once per paragraph.
fn para_end(b: &[u8], from: usize, memo: &mut Memo) -> usize {
    if let Some(end) = memo.para_end
        && end > from
    {
        return end;
    }
    let mut k = from;
    let end = loop {
        if k >= b.len() {
            break b.len();
        }
        if b[k] == b'\n' {
            let mut m = k + 1;
            while m < b.len() && matches!(b[m], b' ' | b'\t' | b'\r') {
                m += 1;
            }
            if m >= b.len() || b[m] == b'\n' {
                break k;
            }
        }
        k += 1;
    };
    memo.para_end = Some(end);
    end
}

/// Inline `$...$` starting at `i` (a single `$`). Returns the index of the closer.
/// `limit` is the end of the current line.
fn try_inline_dollar(b: &[u8], i: usize, limit: usize) -> Option<usize> {
    let first = *b.get(i + 1)?;
    if first.is_ascii_whitespace() {
        return None;
    }
    let mut j = i + 1;
    while j < limit {
        match b[j] {
            b'\\' => j += 2,
            b'$' => {
                let prev = b[j - 1];
                let next_is_digit = b.get(j + 1).is_some_and(|c| c.is_ascii_digit());
                if j > i + 1 && !prev.is_ascii_whitespace() && !next_is_digit {
                    // A digit-led span is math only when it has a formula marker or a letter:
                    // `$5-$6` and `$10$` are prices, `$2^n$` and `$3x$` are formulas.
                    if first.is_ascii_digit() && !is_formulaic(&b[i + 1..j]) {
                        return None;
                    }
                    return Some(j);
                }
                j += 1;
            }
            _ => j += 1,
        }
    }
    None
}

/// Whether a span body that starts with a digit is formula-like: it has a letter or one of the
/// operators `^ _ \ { } =`.
fn is_formulaic(body: &[u8]) -> bool {
    body.iter()
        .any(|&c| c.is_ascii_alphabetic() || matches!(c, b'^' | b'_' | b'\\' | b'{' | b'}' | b'='))
}

/// Closing `$$` for display math, searching from `from` up to `limit`.
fn find_close_dollar_dollar(b: &[u8], from: usize, limit: usize) -> Option<usize> {
    let end = limit.min(b.len());
    let mut j = from;
    while j + 1 < end {
        match b[j] {
            b'\\' => j += 2,
            b'$' if b[j + 1] == b'$' => return Some(j),
            _ => j += 1,
        }
    }
    None
}

/// Closing `\)` (or `\]` when `display`) from `from` up to `limit`. Skips escaped pairs.
fn find_close_backslash(b: &[u8], from: usize, limit: usize, display: bool) -> Option<usize> {
    let close = if display { b']' } else { b')' };
    let end = limit.min(b.len());
    let mut j = from;
    while j + 1 < end {
        if b[j] == b'\\' {
            if b[j + 1] == close {
                return Some(j);
            }
            j += 2;
        } else {
            j += 1;
        }
    }
    None
}

fn run_len(b: &[u8], from: usize, ch: u8) -> usize {
    b[from..].iter().take_while(|&&c| c == ch).count()
}

/// Index just past the closing backtick run of exactly `len` backticks.
fn find_backtick_run(b: &[u8], from: usize, len: usize) -> Option<usize> {
    let mut j = from;
    while j < b.len() {
        if b[j] == b'`' {
            let run = run_len(b, j, b'`');
            if run == len {
                return Some(j + run);
            }
            j += run;
        } else {
            j += 1;
        }
    }
    None
}

/// If a fenced code block opens at line start `i`, return the index just past it.
fn fenced_block_end(b: &[u8], i: usize) -> Option<usize> {
    let first_end = line_end(b, i);
    let line = &b[i..first_end];
    let indent = line.iter().take_while(|&&c| c == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let fence = *rest.first()?;
    if fence != b'`' && fence != b'~' {
        return None;
    }
    let len = run_len(rest, 0, fence);
    if len < 3 {
        return None;
    }
    // Scan subsequent lines for a closing fence of the same char, length >= len.
    let mut k = (first_end + 1).min(b.len());
    while k < b.len() {
        let end = line_end(b, k);
        let l = &b[k..end];
        let ind = l.iter().take_while(|&&c| c == b' ').count();
        if ind <= 3 {
            let r = &l[ind..];
            let run = run_len(r, 0, fence);
            if run >= len && r[run..].iter().all(|c| c.is_ascii_whitespace()) {
                return Some((end + 1).min(b.len()));
            }
        }
        k = end + 1;
    }
    // Unterminated fence runs to the end of the document (CommonMark).
    Some(b.len())
}

/// Index of the `\n` ending the line that contains `from`, or the text length.
fn line_end(b: &[u8], from: usize) -> usize {
    b[from..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latex_list(text: &str) -> Vec<(String, bool)> {
        find_math_spans(text)
            .into_iter()
            .map(|s| (s.latex, s.display))
            .collect()
    }

    #[test]
    fn inline_and_display_basics() {
        assert_eq!(latex_list("a $x^2$ b"), vec![("x^2".to_string(), false)]);
        assert_eq!(
            latex_list("$$\\sum_{i}$$"),
            vec![("\\sum_{i}".to_string(), true)]
        );
        assert_eq!(
            latex_list(r"\(a+b\) and \[c\]"),
            vec![("a+b".to_string(), false), ("c".to_string(), true)]
        );
    }

    #[test]
    fn ranges_cover_delimiters() {
        let text = "x $a$ y";
        let spans = find_math_spans(text);
        assert_eq!(spans.len(), 1);
        assert_eq!(&text[spans[0].range.clone()], "$a$");
    }

    #[test]
    fn dollar_followed_by_space_does_not_open() {
        assert!(find_math_spans("price $ 5 and $ 6").is_empty());
    }

    #[test]
    fn closing_dollar_preceded_by_space_does_not_close() {
        assert!(find_math_spans("$a $ b").is_empty());
    }

    #[test]
    fn currency_is_not_math() {
        assert!(find_math_spans("costs $5 and $10 total").is_empty());
        assert!(find_math_spans("it is $5").is_empty());
    }

    #[test]
    fn digit_led_math_with_operators_is_math() {
        // `$2^n$` is a formula: the digit guard only rejects prices (digits and punctuation).
        assert_eq!(
            latex_list("size $2^n$ here"),
            vec![("2^n".to_string(), false)]
        );
        assert_eq!(latex_list("$3x$"), vec![("3x".to_string(), false)]);
    }

    #[test]
    fn prices_with_a_closing_dollar_stay_text() {
        assert!(find_math_spans("pay $5 and $6 today").is_empty());
        assert!(find_math_spans("range $5-$6 today").is_empty());
        assert!(find_math_spans("costs $10$ only").is_empty());
    }

    #[test]
    fn escaped_dollar_is_text() {
        assert!(find_math_spans(r"a \$x\$ b").is_empty());
        // An escaped backslash does not escape the closing dollar.
        assert_eq!(latex_list(r"$a\\$"), vec![(r"a\\".to_string(), false)]);
    }

    #[test]
    fn code_spans_are_skipped() {
        assert!(find_math_spans("`$x$`").is_empty());
        assert!(find_math_spans("``a $x$ b``").is_empty());
        assert_eq!(latex_list("`code` $y$"), vec![("y".to_string(), false)]);
    }

    #[test]
    fn fenced_code_blocks_are_skipped() {
        let text = "```\n$x$\n```\n$y$";
        assert_eq!(latex_list(text), vec![("y".to_string(), false)]);
        let tilde = "~~~python\n$z$\n~~~\n$w$";
        assert_eq!(latex_list(tilde), vec![("w".to_string(), false)]);
    }

    #[test]
    fn unterminated_fence_hides_the_rest() {
        assert!(find_math_spans("```\n$x$ and more").is_empty());
    }

    #[test]
    fn inline_does_not_cross_lines_but_display_may() {
        assert!(find_math_spans("$a\nb$").is_empty());
        assert_eq!(latex_list("$$a\nb$$"), vec![("a\nb".to_string(), true)]);
    }

    #[test]
    fn display_does_not_cross_blank_line() {
        assert!(find_math_spans("$$a\n\nb$$").is_empty());
    }

    #[test]
    fn cjk_around_math_keeps_byte_ranges_valid() {
        let text = "中文 $\\alpha$ 测试 $$\\int$$ 结束";
        let spans = find_math_spans(text);
        assert_eq!(spans.len(), 2);
        assert_eq!(&text[spans[0].range.clone()], "$\\alpha$");
        assert_eq!(&text[spans[1].range.clone()], "$$\\int$$");
    }

    #[test]
    fn empty_math_is_ignored() {
        assert!(find_math_spans("$$ $$").is_empty());
        assert!(find_math_spans("$$$$").is_empty());
    }

    #[test]
    fn unclosed_delimiters_are_text() {
        assert!(find_math_spans("$x").is_empty());
        assert!(find_math_spans("$$x").is_empty());
        assert!(find_math_spans(r"\(x").is_empty());
    }

    #[test]
    fn multiple_spans_in_order() {
        let spans = find_math_spans("$a$ $b$ $$c$$");
        let latex: Vec<_> = spans.iter().map(|s| s.latex.as_str()).collect();
        assert_eq!(latex, vec!["a", "b", "c"]);
    }

    #[test]
    fn pathological_input_finishes_quickly() {
        let started = std::time::Instant::now();
        let dollars = "$".repeat(200_000);
        assert!(find_math_spans(&dollars).is_empty());
        let words = "$a ".repeat(60_000);
        assert!(find_math_spans(&words).is_empty());
        let fences = "```\n".repeat(20_000);
        let _ = find_math_spans(&fences);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
