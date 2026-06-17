# Embedded fonts

These are **subsets** of [DejaVu](https://dejavu-fonts.github.io/) faces, embedded into
every exported PDF (see `../src/fonts.rs`). We embed a real Unicode TrueType font rather than
relying on the PDF base-14 fonts so umlauts, dashes, arrows and common math symbols render
directly — no Win-1252 transliteration, and no dependency on the viewer's or the runtime
image's installed fonts (`debian:trixie-slim` ships none).

| File                         | Face                         |
|------------------------------|------------------------------|
| `DejaVuSans.ttf`             | DejaVu Sans (regular)        |
| `DejaVuSans-Bold.ttf`        | DejaVu Sans Bold             |
| `DejaVuSans-Oblique.ttf`     | DejaVu Sans Oblique (italic) |
| `DejaVuSans-BoldOblique.ttf` | DejaVu Sans Bold Oblique     |
| `DejaVuSansMono.ttf`         | DejaVu Sans Mono (code)      |

Because printpdf is built with `default-features = false` (no `text_layout`), it embeds the
font bytes **as-is** with no runtime subsetting. We therefore subset *offline* so each face is
~35–40 KB instead of ~700 KB.

## Regenerating

Source faces come from Debian's `fonts-dejavu-core` + `fonts-dejavu-extra`
(`/usr/share/fonts/truetype/dejavu/`). With `fonttools` installed (`pyftsubset`):

```sh
SRC=/usr/share/fonts/truetype/dejavu
RANGES="U+0020-007E,U+00A0-024F,U+2010-2027,U+20AC,U+2190-2194,U+21D0-21D2,U+2212,U+2248,U+2260,U+2264,U+2265"
for f in DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSans-BoldOblique.ttf DejaVuSansMono.ttf; do
  pyftsubset "$SRC/$f" --unicodes="$RANGES" --layout-features="" --no-hinting --desubroutinize \
    --output-file="$f"
done
```

Coverage: Basic Latin, Latin-1 Supplement + Latin Extended-A/B start, General Punctuation
(dashes, smart quotes, ellipsis, bullet, dagger), the Euro sign, the four cardinal + the
horizontal arrows, double arrows `⇐ ⇒`, and the common math operators `− ≈ ≠ ≤ ≥`. Code
points outside this set degrade to `?` via the guard in `src/text.rs`; widen `RANGES` if real
reports need more.

`LICENSE` is the DejaVu / Bitstream Vera license (permissive, redistribution permitted).
