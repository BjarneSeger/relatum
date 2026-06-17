//! Markdown → layout blocks.
//!
//! A pragmatic pass over the `pulldown-cmark` event stream (same extensions the web
//! frontend enables) that flattens a report body into [`Block`]s the renderer flows
//! into the activities area. Inline emphasis becomes face switches; lists, code,
//! rules and blockquotes are supported; tables degrade to one monospace line per row
//! and images are dropped (report bodies are short prose, not rich documents).

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::fonts::Style;
use crate::text::{Run, sanitize};

/// A laid-out block of content.
#[derive(Debug)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Run>,
    },
    Paragraph(Vec<Run>),
    /// A list item: `depth` is its nesting level (0 = top), `marker` the bullet or
    /// number prefix, `spans` the item's inline content.
    ListItem {
        depth: u8,
        marker: String,
        spans: Vec<Run>,
    },
    /// A fenced/indented code block, one entry per source line.
    Code(Vec<String>),
    /// A blockquote paragraph.
    Quote(Vec<Run>),
    Rule,
}

/// Nesting context for an open list.
struct ListCtx {
    ordered: bool,
    next: u64,
}

/// Parse `markdown` into a sequence of layout blocks.
pub fn to_blocks(markdown: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut blocks: Vec<Block> = Vec::new();

    // Inline accumulation state.
    let mut spans: Vec<Run> = Vec::new();
    let mut bold = 0u32;
    let mut italic = 0u32;

    // Block context.
    let mut lists: Vec<ListCtx> = Vec::new();
    let mut in_item = false;
    let mut in_quote = false;
    let mut pending_marker: Option<String> = None;

    // Code block state.
    let mut in_code = false;
    let mut code = String::new();

    // Table state: collect cell text into the current row.
    let mut in_table = false;
    let mut row: String = String::new();
    let mut cell_started = false;

    let push_text = |spans: &mut Vec<Run>, bold: u32, italic: u32, text: &str| {
        let style = Style::from_emphasis(bold > 0, italic > 0);
        spans.push(Run::new(style, sanitize(style, text)));
    };

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !spans.is_empty() {
                    let taken = std::mem::take(&mut spans);
                    blocks.push(if in_quote {
                        Block::Quote(taken)
                    } else {
                        Block::Paragraph(taken)
                    });
                }
            }

            Event::Start(Tag::Heading { level, .. }) => {
                spans.clear();
                pending_marker = Some(heading_num(level).to_string());
            }
            Event::End(TagEnd::Heading(_)) => {
                let level = pending_marker
                    .take()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                blocks.push(Block::Heading {
                    level,
                    spans: std::mem::take(&mut spans),
                });
            }

            Event::Start(Tag::List(start)) => {
                lists.push(ListCtx {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                in_item = true;
                spans.clear();
                let depth = lists.len().saturating_sub(1) as u8;
                let marker = match lists.last_mut() {
                    Some(ctx) if ctx.ordered => {
                        let m = format!("{}.", ctx.next);
                        ctx.next += 1;
                        m
                    }
                    _ => "•".to_string(),
                };
                pending_marker = Some(format!("{depth}\u{1}{marker}"));
            }
            Event::End(TagEnd::Item) => {
                in_item = false;
                let (depth, marker) = pending_marker
                    .take()
                    .and_then(|s| {
                        s.split_once('\u{1}')
                            .map(|(d, m)| (d.parse().unwrap_or(0), m.to_string()))
                    })
                    .unwrap_or((0, "•".to_string()));
                blocks.push(Block::ListItem {
                    depth,
                    marker,
                    spans: std::mem::take(&mut spans),
                });
            }

            Event::Start(Tag::BlockQuote(_)) => in_quote = true,
            Event::End(TagEnd::BlockQuote(_)) => in_quote = false,

            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                code.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                let lines = code
                    .trim_end_matches('\n')
                    .split('\n')
                    .map(|line| sanitize(Style::Mono, line))
                    .collect();
                blocks.push(Block::Code(lines));
            }

            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),

            // Tables degrade to one monospace line per row, cells joined by " | ".
            Event::Start(Tag::Table(_)) => in_table = true,
            Event::End(TagEnd::Table) => in_table = false,
            Event::Start(Tag::TableRow) | Event::Start(Tag::TableHead) => {
                row.clear();
                cell_started = false;
            }
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                if !row.is_empty() {
                    blocks.push(Block::Code(vec![std::mem::take(&mut row)]));
                }
            }
            Event::Start(Tag::TableCell) => {
                if cell_started {
                    row.push_str(" | ");
                }
                cell_started = true;
            }
            Event::End(TagEnd::TableCell) => {}

            Event::Text(text) => {
                if in_code {
                    code.push_str(&text);
                } else if in_table {
                    row.push_str(&sanitize(Style::Mono, &text));
                } else {
                    push_text(&mut spans, bold, italic, &text);
                }
            }
            Event::Code(text) => {
                if in_table {
                    row.push_str(&sanitize(Style::Mono, &text));
                } else {
                    spans.push(Run::new(Style::Mono, sanitize(Style::Mono, &text)));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code {
                    code.push('\n');
                } else if in_table {
                    row.push(' ');
                } else {
                    spans.push(Run::new(Style::Regular, " "));
                }
            }
            Event::Rule => blocks.push(Block::Rule),
            Event::TaskListMarker(done) => {
                spans.push(Run::new(Style::Regular, if done { "[x] " } else { "[ ] " }));
            }

            _ => {}
        }
    }

    // A trailing paragraph with no closing event (defensive).
    if !spans.is_empty() {
        blocks.push(Block::Paragraph(spans));
    }
    let _ = in_item; // state tracked for clarity; items flush on End(Item)
    blocks
}

fn heading_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}
