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
        hershey_simplex::for_each_segment(ch, |start, end| {
            let p0 = place(
                Point2::new(start.0, start.1),
                x_cursor,
                height,
                factor,
                shear,
                origin,
                sin,
                cos,
            );
            let p1 = place(
                Point2::new(end.0, end.1),
                x_cursor,
                height,
                factor,
                shear,
                origin,
                sin,
                cos,
            );
            segments.push([p0, p1]);
        });
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

// ------------------------------------------------------------
// Function: expand_cad_codes
// Purpose: AutoCAD %%d %%p %%c %%nnn and \U+XXXX become glyphs.
//          %%u and %%o are underline and overline toggles with no
//          stroke of their own, so they are consumed.
// ------------------------------------------------------------
pub fn expand_cad_codes(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        if is_unicode_escape(&chars, i) {
            let (next, ch) = read_unicode(&chars, i + 3);
            if let Some(ch) = ch {
                out.push(ch);
                i = next;
                continue;
            }
        }
        if chars[i] == '%' && i + 2 < chars.len() && chars[i + 1] == '%' {
            match chars[i + 2] {
                'd' | 'D' => out.push('\u{00B0}'),
                'p' | 'P' => out.push('\u{00B1}'),
                'c' | 'C' => out.push('\u{2300}'),
                '%' => out.push('%'),
                'u' | 'U' | 'o' | 'O' => {}
                '0'..='9' => {
                    if let Some((next, ch)) = read_percent_digits(&chars, i + 2) {
                        if let Some(ch) = ch {
                            out.push(ch);
                        }
                        i = next;
                        continue;
                    }
                    out.push('%');
                    out.push('%');
                    out.push(chars[i + 2]);
                }
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

fn is_unicode_escape(chars: &[char], i: usize) -> bool {
    i + 2 < chars.len() && chars[i] == '\\' && chars[i + 1] == 'U' && chars[i + 2] == '+'
}

fn read_unicode(chars: &[char], mut i: usize) -> (usize, Option<char>) {
    let start = i;
    while i < chars.len() && i - start < 6 && chars[i].is_ascii_hexdigit() {
        i += 1;
    }
    let digits: String = chars[start..i].iter().collect();
    let ch = u32::from_str_radix(&digits, 16)
        .ok()
        .and_then(char::from_u32);
    (i, ch.filter(|_| !digits.is_empty()))
}

fn read_percent_digits(chars: &[char], start: usize) -> Option<(usize, Option<char>)> {
    if start + 3 > chars.len() {
        return None;
    }
    let mut code = 0u32;
    for offset in 0..3 {
        let digit = chars[start + offset].to_digit(10)?;
        code = code * 10 + digit;
    }
    Some((start + 3, char::from_u32(code)))
}

// ------------------------------------------------------------
// Function: strip_mtext
// Purpose: Drop MTEXT formatting and keep the visible characters.
//          \P and \N break the line. \p...; is a paragraph property
//          and is skipped. Stacked \S fractions become a/b.
// ------------------------------------------------------------
pub fn strip_mtext(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'P' | 'N' | 'X' => {
                    out.push('\n');
                    i += 2;
                }
                'p' => i = skip_semicolon(&chars, i + 2),
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
                'L' | 'l' | 'O' | 'o' | 'K' | 'k' => i += 2,
                'S' | 's' => i = push_stack(&chars, i + 2, &mut out),
                'U' | 'u' if i + 2 < chars.len() && chars[i + 2] == '+' => {
                    let (next, ch) = read_unicode(&chars, i + 3);
                    if let Some(ch) = ch {
                        out.push(ch);
                    }
                    i = next;
                }
                'A' | 'C' | 'F' | 'H' | 'Q' | 'T' | 'W' | 'f' | 'c' | 'h' => {
                    i = skip_semicolon(&chars, i + 2);
                }
                _ => i += 1,
            }
        } else if chars[i] == '{' || chars[i] == '}' {
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn skip_semicolon(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i] != ';' {
        i += 1;
    }
    if i < chars.len() {
        i + 1
    } else {
        i
    }
}

fn push_stack(chars: &[char], start: usize, out: &mut String) -> usize {
    let end = chars[start..]
        .iter()
        .position(|ch| *ch == ';')
        .map(|offset| start + offset)
        .unwrap_or(chars.len());
    let body = &chars[start..end];
    if let Some(split) = body.iter().position(|ch| matches!(ch, '^' | '/' | '#')) {
        out.extend(body[..split].iter().copied());
        out.push('/');
        out.extend(body[split + 1..].iter().copied());
    } else {
        out.extend(body.iter().copied());
    }
    if end < chars.len() {
        end + 1
    } else {
        end
    }
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
    fn paragraph_justify_and_toggles_do_not_print() {
        assert_eq!(strip_mtext(r"\pxqj;Hail"), "Hail");
        assert_eq!(strip_mtext(r"\LText\l"), "Text");
        assert_eq!(strip_mtext(r"\U+011F"), "ğ");
        assert_eq!(strip_mtext(r"\S1/2;"), "1/2");
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
        assert_eq!(expand_cad_codes("%%u%%o%%065"), "A");
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

    #[test]
    fn simplex_a_has_a_right_stem_and_f_hooks_right() {
        let height = 1.0;
        let a = stroke_text(Point2::new(0.0, 0.0), height, 0.0, "a");
        let stem = a.iter().any(|[p, q]| {
            let top = p.y.max(q.y);
            let bottom = p.y.min(q.y);
            p.x.min(q.x) > 0.6 && top > 0.6 && bottom < 0.2 && (p.x - q.x).abs() < 0.05
        });
        assert!(stem, "a needs the vertical stem on its right side");

        let f = stroke_text(Point2::new(0.0, 0.0), height, 0.0, "f");
        let high_x = f
            .iter()
            .flat_map(|seg| seg.iter())
            .filter(|p| p.y > 0.9)
            .map(|p| p.x)
            .fold(0.0_f64, f64::max);
        let low_x = f
            .iter()
            .flat_map(|seg| seg.iter())
            .filter(|p| p.y < 0.2)
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min);
        assert!(
            high_x > low_x + 0.15,
            "f hook should sit to the right of the stem: {high_x} vs {low_x}"
        );
    }

    #[test]
    fn accented_letters_add_a_mark() {
        let count = |text| stroke_text(Point2::new(0.0, 0.0), 1.0, 0.0, text).len();
        assert!(count("ğ") > count("g"));
        assert!(count("ş") > count("s"));
        assert!(count("ç") > count("c"));
        assert!(count("İ") > count("I"));
        assert!(count("ı") > 0);
        assert!(count("ı") < count("i"), "dotless i drops the tittle");
    }
}
