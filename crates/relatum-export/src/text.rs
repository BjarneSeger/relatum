//! Text sanitization and styled word-wrapping.
//!
//! The bundled DejaVu subsets (see [`crate::fonts`]) cover Latin + German, common
//! typographic punctuation, the cardinal/horizontal arrows and the common math
//! operators — so umlauts, dashes, arrows and `≤`/`≥` render directly with no
//! transliteration. [`sanitize`] only guards against the rare character the chosen face
//! has no glyph for (emoji, CJK, …), mapping it to `?` so report bodies never silently
//! drop a glyph or trip a serializer warning.

use crate::fonts::{self, Fonts, Style};

/// Map a string to what the bundled face for `style` can actually render. Newlines are
/// left untouched (callers strip them); tabs expand to four spaces and other control
/// characters become spaces; any character with no glyph in that face becomes `?`.
pub fn sanitize(style: Style, input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch == '\n' || ch == '\r' {
            out.push(ch);
        } else if ch == '\t' {
            out.push_str("    ");
        } else if ch.is_control() {
            out.push(' ');
        } else if fonts::is_covered(style, ch) {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    out
}

/// A run of text sharing one face. A wrapped line is a sequence of these.
#[derive(Clone, Debug)]
pub struct Run {
    pub style: Style,
    pub text: String,
}

impl Run {
    pub fn new(style: Style, text: impl Into<String>) -> Run {
        Run {
            style,
            text: text.into(),
        }
    }
}

/// Greedily wrap a styled paragraph to `max_width_pt`, preserving each run's face.
///
/// Words are split on ASCII whitespace; a single word may carry several runs (e.g.
/// when emphasis changes mid-word). Returned lines insert a single regular-styled
/// space between words; a word wider than the column is placed on its own line and
/// allowed to overflow (no mid-word hyphenation in v1).
pub fn wrap_runs(fonts: &Fonts, spans: &[Run], size_pt: f32, max_width_pt: f32) -> Vec<Vec<Run>> {
    let words = split_words(spans);
    let space_w = fonts.width_pt(Style::Regular, " ", size_pt);

    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut line: Vec<Run> = Vec::new();
    let mut line_w = 0.0f32;

    for word in words {
        let word_w: f32 = word
            .iter()
            .map(|r| fonts.width_pt(r.style, &r.text, size_pt))
            .sum();
        let extra = if line.is_empty() {
            word_w
        } else {
            space_w + word_w
        };

        if !line.is_empty() && line_w + extra > max_width_pt {
            lines.push(std::mem::take(&mut line));
            line_w = 0.0;
        }

        if !line.is_empty() {
            line.push(Run::new(Style::Regular, " "));
            line_w += space_w;
        }
        line.extend(word);
        line_w += word_w;
    }

    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Break the styled spans into whitespace-separated words, each a list of runs.
fn split_words(spans: &[Run]) -> Vec<Vec<Run>> {
    let mut words: Vec<Vec<Run>> = Vec::new();
    let mut word: Vec<Run> = Vec::new();
    let mut buf = String::new();
    let mut buf_style = Style::Regular;

    let flush_buf = |buf: &mut String, buf_style: Style, word: &mut Vec<Run>| {
        if !buf.is_empty() {
            word.push(Run::new(buf_style, std::mem::take(buf)));
        }
    };

    for run in spans {
        for ch in run.text.chars() {
            if ch.is_whitespace() {
                flush_buf(&mut buf, buf_style, &mut word);
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            } else {
                if !buf.is_empty() && run.style != buf_style {
                    flush_buf(&mut buf, buf_style, &mut word);
                }
                buf_style = run.style;
                buf.push(ch);
            }
        }
    }
    flush_buf(&mut buf, buf_style, &mut word);
    if !word.is_empty() {
        words.push(word);
    }
    words
}
