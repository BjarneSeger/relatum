//! Render a signed report as a PDF in the shape of a German *Ausbildungsnachweis*
//! (weekly training record), entirely from code.
//!
//! The renderer is deliberately decoupled from the rest of relatum: it takes a plain
//! [`ReportDocument`] (owned strings + raw PNG signature bytes) and returns the PDF
//! bytes, so it has no dependency on the domain, storage, or HTTP layers and is
//! trivially unit-testable. The API layer assembles a [`ReportDocument`] from a report
//! and its signatures and calls [`render_report_pdf`].
//!
//! Everything is drawn with the standard PDF base-14 fonts (Helvetica + Courier),
//! which embed no font program and need no font file in the VCS — see [`fonts`]. The
//! report's single markdown `content` field is rendered as one combined activities
//! area; the muster's three captioned sections (*Betriebliche Tätigkeit*,
//! *Unterweisungen…*, *Berufsschule*) are not reproduced because the data model has
//! only one body field. The footer carries the two signature blocks the muster
//! expects: the author (*Auszubildender*) on the left, the signer (*Ausbildender*) on
//! the right (drawn empty until the report is signed).

mod fonts;
mod markdown;
mod text;

use printpdf::{
    Line, LinePoint, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Point, Pt, RawImage, TextItem,
    XObjectTransform,
};

use crate::fonts::{Fonts, Style};
use crate::text::{Run, sanitize, wrap_runs};

/// One party's signature line on the report: their name, their PNG signature image
/// (may be empty if none is on file), and the date shown next to it.
pub struct SignatureBlock {
    pub name: String,
    pub png_bytes: Vec<u8>,
    pub date: String,
}

/// Everything needed to render one report. All text is plain (the caller formats
/// dates and the week range); signature images are raw PNG bytes.
pub struct ReportDocument {
    pub author_name: String,
    pub department: String,
    /// e.g. `"08.06.2026 bis 14.06.2026"`.
    pub week_range: String,
    /// Shown after "Nr." in the header (typically the report id).
    pub report_no: Option<String>,
    /// Shown as "Ausbildungsjahr"; blank when unknown (not in the data model yet).
    pub training_year: Option<String>,
    pub body_markdown: String,
    /// The author (*Auszubildender*); always present, though `png_bytes` may be empty.
    pub author: SignatureBlock,
    /// The signer (*Ausbildender*); `Some` only once the report is signed.
    pub signer: Option<SignatureBlock>,
}

/// Render the report as a finished PDF byte buffer (begins with `%PDF`).
///
/// Infallible: a report should always export. A signature image that cannot be
/// decoded (e.g. a stored value that passed the domain's magic-number check but is not
/// a usable PNG) is simply omitted, leaving its signature line blank.
pub fn render_report_pdf(doc: &ReportDocument) -> Vec<u8> {
    render_inner(doc, true)
}

// ---- page geometry (points; PDF origin is bottom-left, y grows up) ----

const PT_PER_MM: f32 = 72.0 / 25.4;
fn mm(v: f32) -> f32 {
    v * PT_PER_MM
}
const PAGE_W_MM: f32 = 210.0;
const PAGE_H_MM: f32 = 297.0;

// Body text metrics.
const BODY: f32 = 10.5;
const BODY_LH: f32 = 13.5;
const PARA_GAP: f32 = 6.0;
const CODE: f32 = 9.0;
const CODE_LH: f32 = 11.0;
const LIST_INDENT: f32 = 14.0;

fn render_inner(doc: &ReportDocument, optimize: bool) -> Vec<u8> {
    let mut pdf = PdfDocument::new("Ausbildungsnachweis");
    // Parse and register the bundled DejaVu faces against this document up front; the
    // renderer holds the resulting font handles.
    let fonts = Fonts::load(&mut pdf);
    let mut r = Renderer::new(&fonts);

    r.begin_page(true);
    draw_page1_chrome(&mut r, &mut pdf, doc);
    flow_blocks(&mut r, &markdown::to_blocks(&doc.body_markdown));
    let pages = r.finish();

    pdf.with_pages(pages);
    let mut warnings = Vec::new();
    pdf.save(
        &PdfSaveOptions {
            optimize,
            // `subset_fonts` is a no-op without printpdf's `text_layout` feature, so each
            // used face embeds the (already offline-subset) bundled bytes as-is. Only the
            // faces actually shown are embedded.
            ..Default::default()
        },
        &mut warnings,
    )
}

/// Accumulates content operations for the current page and paginates the body flow.
struct Renderer<'f> {
    fonts: &'f Fonts,
    pages: Vec<PdfPage>,
    ops: Vec<Op>,
    // Active body text frame (points).
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    cur_y: f32,
    started: bool,
}

impl<'f> Renderer<'f> {
    fn new(fonts: &'f Fonts) -> Renderer<'f> {
        Renderer {
            fonts,
            pages: Vec::new(),
            ops: Vec::new(),
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            cur_y: 0.0,
            started: false,
        }
    }

    /// Finish the current page (if any) and start a fresh one. The first page carries
    /// the header/footer chrome (added by [`draw_page1_chrome`]); later pages get just
    /// the activities frame and a "(Fortsetzung)" label.
    fn begin_page(&mut self, first: bool) {
        if self.started {
            self.flush_page();
        }
        self.ops = vec![Op::SetOutlineThickness { pt: Pt(0.7) }];
        self.started = true;

        if first {
            // Activities box sits between the header and the footer.
            self.rect(mm(20.0), mm(62.0), mm(170.0), mm(188.0));
            self.left = mm(24.0);
            self.right = mm(186.0);
            self.top = mm(246.0);
            self.bottom = mm(66.0);
        } else {
            self.rect(mm(20.0), mm(20.0), mm(170.0), mm(261.0));
            self.text_run(mm(20.0), mm(285.0), Style::Italic, 9.0, "(Fortsetzung)");
            self.left = mm(24.0);
            self.right = mm(186.0);
            self.top = mm(277.0);
            self.bottom = mm(24.0);
        }
        self.cur_y = self.top - 12.0;
    }

    fn flush_page(&mut self) {
        let ops = std::mem::take(&mut self.ops);
        self.pages
            .push(PdfPage::new(Mm(PAGE_W_MM), Mm(PAGE_H_MM), ops));
    }

    fn finish(mut self) -> Vec<PdfPage> {
        if self.started {
            self.flush_page();
        }
        self.pages
    }

    /// Draw a stroked rectangle given its lower-left corner and size (points).
    ///
    /// printpdf 0.9's `Op::DrawRectangle` ignores its `PaintMode` and only emits a
    /// clipping path (`re W n`); several in a row would intersect to an empty clip and
    /// hide the whole page. So we stroke the outline as a closed line, which printpdf
    /// renders correctly (the same path `hline` uses).
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
        self.ops.push(Op::DrawLine {
            line: Line {
                points: corners
                    .iter()
                    .map(|&(px, py)| LinePoint {
                        p: Point {
                            x: Pt(px),
                            y: Pt(py),
                        },
                        bezier: false,
                    })
                    .collect(),
                is_closed: true,
            },
        });
    }

    /// Draw a horizontal line at height `y` from `x1` to `x2` (points).
    fn hline(&mut self, x1: f32, x2: f32, y: f32) {
        self.ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: Point {
                            x: Pt(x1),
                            y: Pt(y),
                        },
                        bezier: false,
                    },
                    LinePoint {
                        p: Point {
                            x: Pt(x2),
                            y: Pt(y),
                        },
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
    }

    /// Draw one line of styled runs with its baseline at `(x, baseline)` (points).
    /// Runs flow left-to-right: after BT the cursor is absolute, and each `ShowText`
    /// advances the text position by the WinAnsi metrics of the named font.
    fn text_line(&mut self, x: f32, baseline: f32, runs: &[Run], size: f32) {
        self.ops.push(Op::StartTextSection);
        self.ops.push(Op::SetTextCursor {
            pos: Point {
                x: Pt(x),
                y: Pt(baseline),
            },
        });
        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            // Select the run's embedded face, then show its text. printpdf maps each
            // character to a glyph id through the face's cmap and emits the Identity-H
            // glyph string; `sanitize` is the last guard against a character the face has
            // no glyph for (chrome text is not pre-sanitized like the markdown body is).
            let font = self.fonts.handle(run.style);
            self.ops.push(Op::SetFont {
                font,
                size: Pt(size),
            });
            self.ops.push(Op::ShowText {
                items: vec![TextItem::Text(sanitize(run.style, &run.text))],
            });
        }
        self.ops.push(Op::EndTextSection);
    }

    /// A single-run convenience over [`Self::text_line`].
    fn text_run(&mut self, x: f32, baseline: f32, style: Style, size: f32, text: &str) {
        let run = Run::new(style, text);
        self.text_line(x, baseline, std::slice::from_ref(&run), size);
    }

    /// Move the cursor down without a page break (inter-block spacing).
    fn gap(&mut self, dy: f32) {
        self.cur_y -= dy;
    }

    /// Ensure a line of height `line_h` fits below the cursor, breaking to a
    /// continuation page if not.
    fn ensure(&mut self, line_h: f32) {
        if self.cur_y - line_h < self.bottom {
            self.begin_page(false);
        }
    }

    /// Wrap and emit `spans` between `left`..`right`, paginating as needed.
    fn emit_wrapped(&mut self, spans: &[Run], size: f32, line_h: f32, left: f32, right: f32) {
        let fonts = self.fonts; // copy the shared ref so measuring won't borrow `self`
        let lines = wrap_runs(fonts, spans, size, right - left);
        for line in &lines {
            self.ensure(line_h);
            let y = self.cur_y;
            self.text_line(left, y, line, size);
            self.cur_y -= line_h;
        }
    }
}

/// Re-style every run to a bold (or bold-italic) face, for headings.
fn force_bold(spans: &[Run]) -> Vec<Run> {
    spans
        .iter()
        .map(|r| {
            let style = match r.style {
                Style::Italic | Style::BoldItalic => Style::BoldItalic,
                _ => Style::Bold,
            };
            Run::new(style, r.text.clone())
        })
        .collect()
}

/// Re-style every prose run to italic, for blockquotes (monospace is left alone).
fn force_italic(spans: &[Run]) -> Vec<Run> {
    spans
        .iter()
        .map(|r| {
            let style = match r.style {
                Style::Bold | Style::BoldItalic => Style::BoldItalic,
                Style::Mono => Style::Mono,
                _ => Style::Italic,
            };
            Run::new(style, r.text.clone())
        })
        .collect()
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 15.0,
        2 => 12.5,
        3 => 11.5,
        _ => BODY,
    }
}

/// Flow the parsed markdown blocks into the activities area.
fn flow_blocks(r: &mut Renderer, blocks: &[markdown::Block]) {
    use markdown::Block;
    let mut first = true;
    for block in blocks {
        match block {
            Block::Heading { level, spans } => {
                if !first {
                    r.gap(8.0);
                }
                let size = heading_size(*level);
                r.emit_wrapped(&force_bold(spans), size, size * 1.3, r.left, r.right);
                r.gap(4.0);
            }
            Block::Paragraph(spans) => {
                r.emit_wrapped(spans, BODY, BODY_LH, r.left, r.right);
                r.gap(PARA_GAP);
            }
            Block::Quote(spans) => {
                let left = r.left + LIST_INDENT;
                r.emit_wrapped(&force_italic(spans), BODY, BODY_LH, left, r.right);
                r.gap(PARA_GAP);
            }
            Block::ListItem {
                depth,
                marker,
                spans,
            } => {
                let fonts = r.fonts;
                let indent = r.left + f32::from(*depth) * LIST_INDENT;
                let marker_w = fonts.width_pt(Style::Regular, marker, BODY) + 4.0;
                let text_left = indent + marker_w;
                let lines = wrap_runs(fonts, spans, BODY, r.right - text_left);
                for (i, line) in lines.iter().enumerate() {
                    r.ensure(BODY_LH);
                    let y = r.cur_y;
                    if i == 0 {
                        r.text_run(indent, y, Style::Regular, BODY, marker);
                    }
                    r.text_line(text_left, y, line, BODY);
                    r.cur_y -= BODY_LH;
                }
                r.gap(3.0);
            }
            Block::Code(lines) => {
                for line in lines {
                    r.ensure(CODE_LH);
                    let y = r.cur_y;
                    r.text_run(r.left + 6.0, y, Style::Mono, CODE, line);
                    r.cur_y -= CODE_LH;
                }
                r.gap(PARA_GAP);
            }
            Block::Rule => {
                r.ensure(8.0);
                r.gap(4.0);
                let y = r.cur_y;
                r.hline(r.left, r.right, y);
                r.gap(6.0);
            }
        }
        first = false;
    }
}

/// Draw the page-1 header box and the footer signature blocks, stamping the
/// signature images. Only this needs the document (to register image XObjects).
fn draw_page1_chrome(r: &mut Renderer, pdf: &mut PdfDocument, d: &ReportDocument) {
    // Header box.
    r.rect(mm(20.0), mm(255.0), mm(170.0), mm(26.0));
    let hx = mm(23.0);
    let nr = d.report_no.as_deref().unwrap_or("");
    let title = vec![
        Run::new(Style::Bold, "Ausbildungsnachweis"),
        Run::new(Style::Regular, format!("   Nr. {nr}")),
    ];
    r.text_line(hx, mm(275.0), &title, 11.5);
    r.text_run(
        hx,
        mm(269.0),
        Style::Regular,
        10.5,
        &format!("Name, Vorname: {}", d.author_name),
    );
    r.text_run(
        hx,
        mm(263.0),
        Style::Regular,
        10.5,
        &format!("Abteilung oder Arbeitsgebiet: {}", d.department),
    );
    let mut week_line = format!("für die Woche vom {}", d.week_range);
    if let Some(year) = &d.training_year
        && !year.is_empty()
    {
        week_line.push_str(&format!("      Ausbildungsjahr: {year}"));
    }
    r.text_run(hx, mm(257.0), Style::Regular, 10.5, &week_line);

    // Footer signature boxes.
    draw_signature_box(r, pdf, mm(20.0), &d.author, "Unterschrift Auszubildender");
    let empty = SignatureBlock {
        name: String::new(),
        png_bytes: Vec::new(),
        date: String::new(),
    };
    let signer = d.signer.as_ref().unwrap_or(&empty);
    draw_signature_box(r, pdf, mm(109.0), signer, "Unterschrift Ausbildender");
}

/// One footer signature box at horizontal offset `x0` (points), 81mm wide.
fn draw_signature_box(
    r: &mut Renderer,
    pdf: &mut PdfDocument,
    x0: f32,
    block: &SignatureBlock,
    caption: &str,
) {
    let w = mm(81.0);
    r.rect(x0, mm(18.0), w, mm(38.0));
    let pad = mm(3.0);

    if !block.date.is_empty() {
        r.text_run(
            x0 + pad,
            mm(51.0),
            Style::Regular,
            9.0,
            &format!("Datum: {}", block.date),
        );
    }
    if !block.name.is_empty() {
        // The signer's name, small, just under the date.
        r.text_run(x0 + pad, mm(45.0), Style::Regular, 8.5, &block.name);
    }

    // Signature image, fitted into the area above the rule. An undecodable image is
    // skipped (the line just stays blank) rather than failing the whole export.
    if !block.png_bytes.is_empty() {
        let img_left = x0 + mm(4.0);
        let img_bottom = mm(30.0);
        let img_w = w - mm(8.0);
        let img_h = mm(13.0);
        stamp_image(r, pdf, &block.png_bytes, img_left, img_bottom, img_w, img_h);
    }

    // Rule and caption.
    r.hline(x0 + pad, x0 + w - pad, mm(28.0));
    r.text_run(x0 + pad, mm(22.0), Style::Regular, 8.5, caption);
}

/// Decode `png` and place it, scaled to fit `(w, h)` preserving aspect, with its
/// lower-left near `(left, bottom)` and centered horizontally in the box. Silently
/// does nothing if the bytes are not a decodable image.
fn stamp_image(
    r: &mut Renderer,
    pdf: &mut PdfDocument,
    png: &[u8],
    left: f32,
    bottom: f32,
    w: f32,
    h: f32,
) {
    let mut warnings = Vec::new();
    let Ok(raw) = RawImage::decode_from_bytes(png, &mut warnings) else {
        return;
    };
    let (iw, ih) = (raw.width as f32, raw.height as f32);
    if iw <= 0.0 || ih <= 0.0 {
        return;
    }
    let id = pdf.add_image(&raw);

    // At dpi = 72, one image pixel maps to one point before our explicit scale.
    let scale = (w / iw).min(h / ih);
    let disp_w = iw * scale;
    let tx = left + (w - disp_w) / 2.0;
    r.ops.push(Op::UseXobject {
        id,
        transform: XObjectTransform {
            translate_x: Some(Pt(tx)),
            translate_y: Some(Pt(bottom)),
            rotate: None,
            scale_x: Some(scale),
            scale_y: Some(scale),
            dpi: Some(72.0),
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a small opaque PNG so tests carry no binary asset.
    fn tiny_png() -> Vec<u8> {
        use image::{ImageFormat, RgbaImage};
        use std::io::Cursor;
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .expect("encode png");
        buf
    }

    fn sig(name: &str, date: &str, png: Vec<u8>) -> SignatureBlock {
        SignatureBlock {
            name: name.into(),
            png_bytes: png,
            date: date.into(),
        }
    }

    fn doc(body: &str, signer: Option<SignatureBlock>) -> ReportDocument {
        ReportDocument {
            author_name: "Müller, Ötzi".into(),
            department: "Fertigung".into(),
            week_range: "08.06.2026 bis 14.06.2026".into(),
            report_no: Some("r-2026-w24".into()),
            training_year: Some("2".into()),
            body_markdown: body.into(),
            author: sig("Müller, Ötzi", "14.06.2026", tiny_png()),
            signer,
        }
    }

    fn count(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    /// Extract page-1 text from our own output. We can't use `lopdf::extract_text` here:
    /// it rejects printpdf's (standard) ToUnicode CMap because of the `/CMapVersion` and
    /// `/WMode` keys its parser doesn't accept. Instead we parse each embedded font's
    /// `bfchar` table (glyph id → char) ourselves and map the Identity-H glyph ids shown
    /// in the content stream back to characters — exactly what a viewer's copy does.
    fn extract_page1_text(bytes: &[u8]) -> String {
        use lopdf::{Document, Object};
        use std::collections::HashMap;

        let doc = Document::load_mem(bytes).expect("our output parses");
        let page = doc.page_iter().next().expect("at least one page");

        let mut cmaps: HashMap<Vec<u8>, HashMap<u16, char>> = HashMap::new();
        for (name, font) in doc.get_page_fonts(page).expect("page fonts") {
            let cmap = font
                .get(b"ToUnicode")
                .and_then(Object::as_reference)
                .and_then(|id| doc.get_object(id))
                .and_then(Object::as_stream)
                .map(|s| s.decompressed_content().unwrap_or_else(|_| s.content.clone()))
                .map(|data| parse_bfchar(&data))
                .unwrap_or_default();
            cmaps.insert(name, cmap);
        }

        let content = doc
            .get_and_decode_page_content(page)
            .expect("decode page content");
        let mut out = String::new();
        let mut cur: Option<Vec<u8>> = None;
        for op in content.operations {
            match op.operator.as_str() {
                "Tf" => cur = op.operands.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec),
                "Tj" => {
                    if let Some(map) = cur.as_ref().and_then(|n| cmaps.get(n))
                        && let Some(s) = op.operands.first().and_then(|o| o.as_str().ok())
                    {
                        push_glyphs(&mut out, s, map);
                    }
                }
                "TJ" => {
                    if let Some(map) = cur.as_ref().and_then(|n| cmaps.get(n))
                        && let Some(Object::Array(arr)) = op.operands.first()
                    {
                        for el in arr {
                            if let Ok(s) = el.as_str() {
                                push_glyphs(&mut out, s, map);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn parse_bfchar(data: &[u8]) -> std::collections::HashMap<u16, char> {
        let text = String::from_utf8_lossy(data);
        let mut map = std::collections::HashMap::new();
        let mut in_block = false;
        for line in text.lines() {
            let line = line.trim();
            if line.ends_with("beginbfchar") {
                in_block = true;
            } else if line == "endbfchar" {
                in_block = false;
            } else if in_block {
                let hex = |t: &str| u32::from_str_radix(t.trim_matches(['<', '>']), 16).ok();
                if let [gid, uni] = line.split_whitespace().collect::<Vec<_>>()[..]
                    && let (Some(gid), Some(ch)) = (hex(gid), hex(uni).and_then(char::from_u32))
                {
                    map.insert(gid as u16, ch);
                }
            }
        }
        map
    }

    fn push_glyphs(out: &mut String, s: &[u8], map: &std::collections::HashMap<u16, char>) {
        for pair in s.chunks_exact(2) {
            if let Some(&ch) = map.get(&u16::from_be_bytes([pair[0], pair[1]])) {
                out.push(ch);
            }
        }
    }

    /// The content-stream operators of page 1, in order (e.g. "re", "S", "Tj").
    fn page1_operators(bytes: &[u8]) -> Vec<String> {
        let doc = lopdf::Document::load_mem(bytes).expect("our output parses");
        let page = doc.page_iter().next().expect("at least one page");
        doc.get_and_decode_page_content(page)
            .expect("decode page content")
            .operations
            .into_iter()
            .map(|op| op.operator)
            .collect()
    }

    #[test]
    fn boxes_are_stroked_not_clipped() {
        // Regression: printpdf 0.9's `DrawRectangle` ignores its paint mode and emits a
        // clipping path; chaining the form boxes that way intersected the clips to an empty
        // region and hid the whole page (text was still present — hence searchable — but
        // never painted). The boxes must be *stroked*, and nothing may set a clip path.
        let bytes = render_report_pdf(&doc(
            "# Woche 24\n\nHeute etwas gemacht.",
            Some(sig("Schmidt, Eva", "16.06.2026", tiny_png())),
        ));
        let ops = page1_operators(&bytes);
        assert!(
            !ops.iter().any(|o| o == "W" || o == "W*"),
            "page must not set a clipping path, got ops: {ops:?}"
        );
        // Four form boxes (activities, header, two signature boxes) plus the two signature
        // rules all stroke, so at least four stroke ops must reach the page.
        let strokes = ops.iter().filter(|o| *o == "S").count();
        assert!(strokes >= 4, "expected the boxes to be stroked, got {strokes} S ops");
    }

    #[test]
    fn renders_a_pdf() {
        let bytes = render_report_pdf(&doc(
            "# Woche 24\n\nHeute **Drehmaschine** eingerichtet und *Werkstücke* gefertigt.",
            Some(sig("Schmidt, Eva", "16.06.2026", tiny_png())),
        ));
        assert!(bytes.starts_with(b"%PDF"), "output is not a PDF");
    }

    #[test]
    fn embeds_subset_font_program() {
        // The bundled DejaVu face is embedded as a Type0/Identity-H CID font with its own
        // font program — not referenced by name and rendered by the viewer.
        let bytes = render_inner(&doc("hello", None), false);
        assert!(count(&bytes, b"/Type0") >= 1, "not a Type0 font");
        assert!(count(&bytes, b"/Identity-H") >= 1, "not Identity-H encoded");
        assert!(count(&bytes, b"/FontFile2") >= 1, "no embedded font program");
        assert!(count(&bytes, b"DejaVuSans") >= 1, "DejaVu face not embedded");
        assert_eq!(
            count(&bytes, b"/Helvetica"),
            0,
            "should not reference a base-14 font"
        );
    }

    #[test]
    fn umlauts_round_trip() {
        // ä ö ü ß render from the embedded font; extracting our own output via its
        // ToUnicode CMap (as any PDF viewer's copy-paste would) must give them back.
        let bytes = render_report_pdf(&doc("Tätigkeit: Öl, Maß, Übung.", None));
        let text = extract_page1_text(&bytes);
        for needle in ["Tätigkeit", "Öl", "Maß", "Übung"] {
            assert!(text.contains(needle), "{needle:?} not found in {text:?}");
        }
    }

    #[test]
    fn symbols_render_without_transliteration() {
        // Arrows and math operators are in the bundled subset, so they survive into the
        // PDF and round-trip through the ToUnicode CMap instead of becoming ASCII.
        let bytes = render_report_pdf(&doc("Schritt → fertig, x ≤ y ≥ z.", None));
        let text = extract_page1_text(&bytes);
        for needle in ["→", "≤", "≥"] {
            assert!(text.contains(needle), "{needle:?} not found in {text:?}");
        }
    }

    #[test]
    fn unsigned_report_renders_with_empty_signer_box() {
        let bytes = render_report_pdf(&doc("Draft body, not yet signed.", None));
        assert!(bytes.starts_with(b"%PDF"));
        // Both signature captions are still drawn — the signer box is just empty.
        let text = extract_page1_text(&bytes);
        assert!(
            text.contains("Unterschrift Auszubildender"),
            "missing author caption"
        );
        assert!(
            text.contains("Unterschrift Ausbildender"),
            "missing signer caption"
        );
    }

    #[test]
    fn embedded_fonts_are_subset_not_full() {
        // The bundled faces are pre-subset (~40 KB each); a full DejaVu face is ~750 KB.
        // A normal report (regular + bold) must stay well under that, or we have regressed
        // to embedding whole fonts.
        let bytes = render_report_pdf(&doc("# Woche\n\nKurzer **Bericht**.", None));
        assert!(
            bytes.len() < 200_000,
            "PDF unexpectedly large ({} bytes) — full font embedded?",
            bytes.len()
        );
    }

    #[test]
    fn long_body_paginates() {
        let para = "Heute an der Anlage gearbeitet und dokumentiert. ".repeat(40);
        let body = (0..30)
            .map(|i| format!("## Abschnitt {i}\n\n{para}\n"))
            .collect::<String>();
        let bytes = render_report_pdf(&doc(&body, None));
        let parsed = lopdf::Document::load_mem(&bytes).expect("our output parses");
        assert!(parsed.get_pages().len() >= 2, "expected multiple pages");
    }

    #[test]
    fn markdown_features_do_not_panic() {
        let body = "\
# Title

- one
- two with `code`
  - nested

1. first
2. second

> a quote

```
let x = 1;
let y = 2;
```

| a | b |
|---|---|
| 1 | 2 |

---

Done with an arrow → and emoji 🚀.";
        let bytes = render_report_pdf(&doc(body, None));
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn sanitize_keeps_covered_replaces_uncovered() {
        use text::sanitize;
        // Arrows and math operators are in the bundled subset — kept verbatim, not
        // transliterated to ASCII.
        assert_eq!(sanitize(Style::Regular, "a→b"), "a→b");
        assert_eq!(sanitize(Style::Regular, "x ≤ y ≥ z"), "x ≤ y ≥ z");
        // Genuinely uncovered code points (emoji) degrade to '?'.
        assert_eq!(sanitize(Style::Regular, "emoji 🚀 here"), "emoji ? here");
        // German + common punctuation survive unchanged.
        assert_eq!(sanitize(Style::Regular, "Maß „ja“ — •"), "Maß „ja“ — •");
    }
}
