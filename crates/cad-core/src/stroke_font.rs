//! Hershey Simplex stroke font for TEXT / MTEXT.
//! Glyphs are cap-height units; `height` is the cap height in world units.

use crate::geom::Point2;
use crate::hershey_simplex;

pub fn measure_width(text: &str, height: f64) -> f64 {
    measure_styled_width(text, height, 1.0)
}

pub fn measure_styled_width(text: &str, height: f64, width_factor: f64) -> f64 {
    let text = expand_cad_codes(text);
    let mut width = 0.0_f64;
    let mut line = 0.0_f64;
    let factor = width_factor.max(0.0);
    for ch in text.chars() {
        if ch == '\n' {
            width = width.max(line);
            line = 0.0;
            continue;
        }
        line += hershey_simplex::advance(ch) * height * factor;
    }
    width.max(line)
}

pub fn stroke_text(origin: Point2, height: f64, rotation: f64, text: &str) -> Vec<[Point2; 2]> {
    stroke_text_styled(origin, height, rotation, 1.0, 0.0, text)
}

// ------------------------------------------------------------
// Function: stroke_text_styled
// Purpose: Place Simplex strokes with width factor and oblique
//          shear, then rotate them about `origin`.
// ------------------------------------------------------------
pub fn stroke_text_styled(
    origin: Point2,
    height: f64,
    rotation: f64,
    width_factor: f64,
    oblique: f64,
    text: &str,
) -> Vec<[Point2; 2]> {
    let text = expand_cad_codes(text);
    let mut segments = Vec::new();
    let (sin, cos) = rotation.sin_cos();
    let shear = oblique.tan();
    let factor = width_factor.max(0.0);
    let mut x_cursor = 0.0;
    for ch in text.chars() {
        if ch == '\n' {
            x_cursor = 0.0;
            continue;
        }
        for stroke in hershey_simplex::strokes(ch).split('|') {
            let points = parse_stroke(stroke);
            for pair in points.windows(2) {
                let p0 = place(pair[0], x_cursor, height, factor, shear, origin, sin, cos);
                let p1 = place(pair[1], x_cursor, height, factor, shear, origin, sin, cos);
                segments.push([p0, p1]);
            }
        }
        x_cursor += hershey_simplex::advance(ch) * height * factor;
    }
    segments
}

fn place(
    glyph: Point2,
    x_cursor: f64,
    height: f64,
    width_factor: f64,
    shear: f64,
    origin: Point2,
    sin: f64,
    cos: f64,
) -> Point2 {
    let x = x_cursor + (glyph.x + glyph.y * shear) * height * width_factor;
    let y = glyph.y * height;
    Point2::new(origin.x + x * cos - y * sin, origin.y + x * sin + y * cos)
}

fn parse_stroke(stroke: &str) -> Vec<Point2> {
    stroke
        .split_whitespace()
        .filter_map(|token| {
            let (x, y) = token.split_once(',')?;
            Some(Point2::new(x.parse().ok()?, y.parse().ok()?))
        })
        .collect()
}

// ------------------------------------------------------------
// Function: expand_cad_codes
// Purpose: AutoCAD %%d %%p %%c %%% control codes become glyphs.
// ------------------------------------------------------------
pub fn expand_cad_codes(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 2 < chars.len() && chars[i + 1] == '%' {
            match chars[i + 2] {
                'd' | 'D' => out.push('\u{00B0}'),
                'p' | 'P' => out.push('\u{00B1}'),
                'c' | 'C' => out.push('\u{2300}'),
                '%' => out.push('%'),
                _ => {
                    out.push('%');
                    out.push('%');
                    out.push(chars[i + 2]);
                }
            }
            i += 3;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

pub fn strip_mtext(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => match chars[i + 1] {
                'P' | 'p' | 'X' | 'x' => {
                    out.push('\n');
                    i += 2;
                }
                '~' => {
                    out.push(' ');
                    i += 2;
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                }
                '{' | '}' => {
                    out.push(chars[i + 1]);
                    i += 2;
                }
                'A' | 'C' | 'F' | 'H' | 'Q' | 'T' | 'W' | 'f' | 'c' | 'h' => {
                    i += 2;
                    while i < chars.len() && chars[i] != ';' {
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                }
                _ => {
                    i += 1;
                }
            },
            '{' | '}' => i += 1,
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtext_strips_formatting_and_paragraphs() {
        let s = strip_mtext(r"{\fArial|b0;Hello\PWorld}");
        assert!(s.contains("Hello"));
        assert!(s.contains('\n'));
        assert!(s.contains("World"));
        assert!(!s.contains('\\'));
    }

    #[test]
    fn digits_emit_strokes() {
        assert!(!stroke_text(Point2::new(0.0, 0.0), 2.5, 0.0, "A1").is_empty());
    }

    #[test]
    fn diameter_code_becomes_a_glyph_and_advances_differ() {
        let expanded = expand_cad_codes("%%c50");
        assert!(expanded.contains('\u{2300}'));
        assert!(!expanded.contains('%'));
        let narrow = measure_width("I", 10.0);
        let wide = measure_width("M", 10.0);
        assert!(
            wide > narrow + 1.0,
            "M should be wider than I: {wide} vs {narrow}"
        );
        let strokes = stroke_text(Point2::new(0.0, 0.0), 10.0, 0.0, "%%c");
        assert!(
            strokes.len() > 2,
            "diameter glyph needs a circle and a slash"
        );
    }
}
