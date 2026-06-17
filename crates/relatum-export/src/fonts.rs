//! Embedded font handling and text measurement.
//!
//! We render with bundled subsets of the DejaVu faces (see `../fonts/`) — DejaVu Sans
//! (regular/bold/oblique/bold-oblique) for prose and DejaVu Sans Mono for code. Each face
//! is embedded into the PDF as a Type0/CID TrueType program with `Identity-H` encoding, so
//! the output renders the same in any viewer and needs no font installed in the runtime
//! image. printpdf (built without `text_layout`) does not parse fonts itself, so we read the
//! `cmap` and glyph advances with `ttf-parser` and hand them to printpdf via
//! [`ParsedFont::with_glyph_data`]; those same advances drive our word-wrapping, so wrapping
//! matches what the viewer lays out. A real font has every glyph it claims (including the
//! space), so there is nothing to approximate.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use printpdf::{FontId, FontMetrics, ParsedFont, PdfDocument, PdfFontHandle};

/// The faces the renderer can switch between. Markdown emphasis maps onto these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Mono,
}

const ALL_STYLES: [Style; 5] = [
    Style::Regular,
    Style::Bold,
    Style::Italic,
    Style::BoldItalic,
    Style::Mono,
];

impl Style {
    fn index(self) -> usize {
        match self {
            Style::Regular => 0,
            Style::Bold => 1,
            Style::Italic => 2,
            Style::BoldItalic => 3,
            Style::Mono => 4,
        }
    }

    /// The bundled subset font program for this face.
    fn font_bytes(self) -> &'static [u8] {
        match self {
            Style::Regular => &include_bytes!("../fonts/DejaVuSans.ttf")[..],
            Style::Bold => &include_bytes!("../fonts/DejaVuSans-Bold.ttf")[..],
            Style::Italic => &include_bytes!("../fonts/DejaVuSans-Oblique.ttf")[..],
            Style::BoldItalic => &include_bytes!("../fonts/DejaVuSans-BoldOblique.ttf")[..],
            Style::Mono => &include_bytes!("../fonts/DejaVuSansMono.ttf")[..],
        }
    }

    /// The `BaseFont` name embedded for this face.
    fn font_name(self) -> &'static str {
        match self {
            Style::Regular => "DejaVuSans",
            Style::Bold => "DejaVuSans-Bold",
            Style::Italic => "DejaVuSans-Oblique",
            Style::BoldItalic => "DejaVuSans-BoldOblique",
            Style::Mono => "DejaVuSansMono",
        }
    }

    /// Combine bold + italic flags into a face.
    pub fn from_emphasis(bold: bool, italic: bool) -> Style {
        match (bold, italic) {
            (true, true) => Style::BoldItalic,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (false, false) => Style::Regular,
        }
    }
}

/// One embedded face: its advance widths (for measuring) and the registered font handle.
struct Face {
    units_per_em: f32,
    /// Advance widths, in font units, keyed by character.
    advances: BTreeMap<char, u16>,
    /// Advance to assume for a character with no glyph (only reached if sanitization lets
    /// an uncovered character through; in practice it is mapped to `?` first).
    fallback: u16,
    /// The embedded font, registered in the document.
    font_id: FontId,
}

impl Face {
    fn load(style: Style, pdf: &mut PdfDocument) -> Face {
        let bytes = style.font_bytes();
        let face = ttf_parser::Face::parse(bytes, 0).expect("bundled DejaVu subset parses");
        let upem = face.units_per_em();

        // Walk the font's Unicode cmap once: build the char->advance table our wrapper uses
        // and the char->glyph / glyph->width maps printpdf needs to emit the embedded font.
        let mut codepoints: Vec<u32> = Vec::new();
        if let Some(cmap) = face.tables().cmap {
            for subtable in cmap.subtables {
                if subtable.is_unicode() {
                    subtable.codepoints(|cp| codepoints.push(cp));
                }
            }
        }

        let mut advances: BTreeMap<char, u16> = BTreeMap::new();
        let mut codepoint_to_glyph: BTreeMap<u32, u16> = BTreeMap::new();
        let mut glyph_widths: BTreeMap<u16, u16> = BTreeMap::new();
        for cp in codepoints {
            if let Some(ch) = char::from_u32(cp)
                && let Some(gid) = face.glyph_index(ch)
            {
                let adv = face.glyph_hor_advance(gid).unwrap_or(0);
                advances.insert(ch, adv);
                codepoint_to_glyph.insert(cp, gid.0);
                glyph_widths.insert(gid.0, adv);
            }
        }

        let fallback = advances
            .get(&'n')
            .copied()
            .unwrap_or((f32::from(upem) * 0.5) as u16);

        // FontDescriptor ascent/descent are expressed per-1000-em.
        let scale = |v: i16| (f32::from(v) * 1000.0 / f32::from(upem)) as i16;
        let parsed = ParsedFont::with_glyph_data(
            bytes.to_vec(),
            0,
            Some(style.font_name().to_string()),
            codepoint_to_glyph,
            glyph_widths,
            upem,
            FontMetrics {
                ascent: scale(face.ascender()),
                descent: scale(face.descender()),
            },
        );
        let font_id = pdf.add_font(&parsed);

        Face {
            units_per_em: f32::from(upem),
            advances,
            fallback,
            font_id,
        }
    }

    fn advance(&self, ch: char) -> u16 {
        self.advances.get(&ch).copied().unwrap_or(self.fallback)
    }
}

/// Loaded faces, registered once per render against the document.
pub struct Fonts {
    faces: Vec<Face>,
}

impl Fonts {
    /// Parse the bundled faces, register them in `pdf`, and cache their advances.
    pub fn load(pdf: &mut PdfDocument) -> Fonts {
        Fonts {
            faces: ALL_STYLES.iter().map(|s| Face::load(*s, pdf)).collect(),
        }
    }

    /// Width of `text` rendered in `style` at `size_pt`, in points.
    pub fn width_pt(&self, style: Style, text: &str, size_pt: f32) -> f32 {
        let face = &self.faces[style.index()];
        let units: u32 = text.chars().map(|c| face.advance(c) as u32).sum();
        units as f32 * size_pt / face.units_per_em
    }

    /// The printpdf handle for this style's embedded font.
    pub fn handle(&self, style: Style) -> PdfFontHandle {
        PdfFontHandle::External(self.faces[style.index()].font_id.clone())
    }
}

/// Per-style glyph coverage of the bundled faces, built lazily from the embedded bytes.
/// Independent of any [`PdfDocument`] so text can be sanitized before rendering.
fn coverage() -> &'static [HashSet<char>; 5] {
    static COVERAGE: OnceLock<[HashSet<char>; 5]> = OnceLock::new();
    COVERAGE.get_or_init(|| ALL_STYLES.map(face_charset))
}

fn face_charset(style: Style) -> HashSet<char> {
    let mut set = HashSet::new();
    if let Ok(face) = ttf_parser::Face::parse(style.font_bytes(), 0)
        && let Some(cmap) = face.tables().cmap
    {
        for subtable in cmap.subtables {
            if subtable.is_unicode() {
                subtable.codepoints(|cp| {
                    if let Some(ch) = char::from_u32(cp) {
                        set.insert(ch);
                    }
                });
            }
        }
    }
    set
}

/// Whether the bundled face for `style` can render `ch`.
pub fn is_covered(style: Style, ch: char) -> bool {
    coverage()[style.index()].contains(&ch)
}
