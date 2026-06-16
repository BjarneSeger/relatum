//! Markdown → sanitized HTML.
//!
//! Report bodies are markdown authored by one user (a trainee) and shown to others
//! (signers, instructors). They are rendered with `pulldown-cmark` and then passed
//! through `ammonia`, which strips scripts, event handlers, and dangerous URLs — so a
//! malicious report body cannot inject script into a reviewer's page. Every template
//! that emits this output marks it `|safe`, which is only sound *because* it has been
//! sanitized here first.

use pulldown_cmark::{Options, Parser, html};

/// Render `markdown` to HTML and sanitize it for safe inline display.
pub fn to_safe_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, options);
    let mut rendered = String::new();
    html::push_html(&mut rendered, parser);
    ammonia::clean(&rendered)
}
