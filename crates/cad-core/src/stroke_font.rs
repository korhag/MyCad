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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MTextAlign {
    Left,
    Center,
    Right,
    Justify,
    Distribute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MTextRun {
    pub text: String,
    pub height: f64,
    pub width_factor: f64,
    /// Empty uses the entity text style. A family name comes from `\f`.
    pub font: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MTextParagraph {
    pub runs: Vec<MTextRun>,
    /// None keeps the MTEXT attachment column.
    pub align: Option<MTextAlign>,
    /// Indents are multiples of the MTEXT height. `first_indent` is added
    /// to the left indent on the first line only.
    pub first_indent: f64,
    pub left_indent: f64,
    pub right_indent: f64,
}

impl MTextParagraph {
    fn plain() -> Self {
        Self {
            runs: Vec::new(),
            align: None,
            first_indent: 0.0,
            left_indent: 0.0,
            right_indent: 0.0,
        }
    }
}

#[derive(Clone)]
struct DrawState {
    height: f64,
    width: f64,
    font: String,
    bold: bool,
    italic: bool,
}

struct ParagraphProps {
    align: Option<MTextAlign>,
    first: Option<f64>,
    left: Option<f64>,
    right: Option<f64>,
}

struct FontSpec {
    family: String,
    bold: bool,
    italic: bool,
}

// ------------------------------------------------------------
// Function: mtext_paragraphs
// Purpose: Split MTEXT into hard paragraphs of styled runs.
//          \H2.5; is an absolute height. \H0.5x; scales the
//          current height. \W works the same way for width.
//          \f switches the font. \p sets alignment and indents.
//          Braces save and restore height, width, and font.
// ------------------------------------------------------------
pub fn mtext_paragraphs(raw: &str, base_height: f64) -> Vec<MTextParagraph> {
    let base = if base_height.is_finite() && base_height > 1e-9 {
        base_height
    } else {
        1.0
    };
    let chars: Vec<char> = raw.chars().collect();
    let mut paragraphs = vec![MTextParagraph::plain()];
    let mut state = DrawState {
        height: base,
        width: 1.0,
        font: String::new(),
        bold: false,
        italic: false,
    };
    let mut saved: Vec<DrawState> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            saved.push(state.clone());
            i += 1;
            continue;
        }
        if chars[i] == '}' {
            if let Some(next) = saved.pop() {
                state = next;
            }
            i += 1;
            continue;
        }
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'P' | 'N' | 'X' => {
                    paragraphs.push(MTextParagraph::plain());
                    i += 2;
                }
                'p' => {
                    let (next, props) = read_paragraph_props(&chars, i + 2);
                    if let Some(paragraph) = paragraphs.last_mut() {
                        if let Some(align) = props.align {
                            paragraph.align = Some(align);
                        }
                        if let Some(value) = props.first {
                            paragraph.first_indent = value;
                        }
                        if let Some(value) = props.left {
                            paragraph.left_indent = value;
                        }
                        if let Some(value) = props.right {
                            paragraph.right_indent = value;
                        }
                    }
                    i = next;
                }
                '~' => {
                    push_mtext_char(&mut paragraphs, ' ', &state);
                    i += 2;
                }
                '\\' => {
                    push_mtext_char(&mut paragraphs, '\\', &state);
                    i += 2;
                }
                '{' | '}' => {
                    push_mtext_char(&mut paragraphs, chars[i + 1], &state);
                    i += 2;
                }
                'L' | 'l' | 'O' | 'o' | 'K' | 'k' => i += 2,
                'S' | 's' => {
                    let mut stacked = String::new();
                    i = push_stack(&chars, i + 2, &mut stacked);
                    push_mtext_str(&mut paragraphs, &stacked, &state);
                }
                'U' | 'u' if i + 2 < chars.len() && chars[i + 2] == '+' => {
                    let (next, ch) = read_unicode(&chars, i + 3);
                    if let Some(ch) = ch {
                        push_mtext_char(&mut paragraphs, ch, &state);
                    }
                    i = next;
                }
                'H' | 'h' => {
                    let (next, parsed) = read_mtext_number(&chars, i + 2);
                    if let Some((value, relative)) = parsed {
                        state.height = if relative {
                            (state.height * value).max(1e-9)
                        } else if value > 1e-9 {
                            value
                        } else {
                            state.height
                        };
                    }
                    i = next;
                }
                'W' | 'w' => {
                    let (next, parsed) = read_mtext_number(&chars, i + 2);
                    if let Some((value, relative)) = parsed {
                        state.width = if relative {
                            (state.width * value).max(1e-9)
                        } else if value > 1e-9 {
                            value
                        } else {
                            state.width
                        };
                    }
                    i = next;
                }
                'F' | 'f' => {
                    let (next, spec) = read_font_spec(&chars, i + 2);
                    if let Some(spec) = spec {
                        state.font = spec.family;
                        state.bold = spec.bold;
                        state.italic = spec.italic;
                    }
                    i = next;
                }
                'A' | 'C' | 'Q' | 'T' | 'c' => i = skip_semicolon(&chars, i + 2),
                _ => i += 1,
            }
        } else {
            push_mtext_char(&mut paragraphs, chars[i], &state);
            i += 1;
        }
    }
    paragraphs
}

fn push_mtext_str(paragraphs: &mut [MTextParagraph], text: &str, state: &DrawState) {
    for ch in text.chars() {
        push_mtext_char(paragraphs, ch, state);
    }
}

fn push_mtext_char(paragraphs: &mut [MTextParagraph], ch: char, state: &DrawState) {
    let Some(runs) = paragraphs.last_mut() else {
        return;
    };
    if let Some(last) = runs.runs.last_mut() {
        if same_run_style(last, state) {
            last.text.push(ch);
            return;
        }
    }
    runs.runs.push(MTextRun {
        text: ch.to_string(),
        height: state.height,
        width_factor: state.width,
        font: state.font.clone(),
        bold: state.bold,
        italic: state.italic,
    });
}

fn same_run_style(run: &MTextRun, state: &DrawState) -> bool {
    (run.height - state.height).abs() <= 1e-9
        && (run.width_factor - state.width).abs() <= 1e-9
        && run.bold == state.bold
        && run.italic == state.italic
        && run.font == state.font
}

fn read_paragraph_props(chars: &[char], mut i: usize) -> (usize, ParagraphProps) {
    let mut props = ParagraphProps {
        align: None,
        first: None,
        left: None,
        right: None,
    };
    while i < chars.len() && chars[i] != ';' {
        match chars[i] {
            ',' | ' ' => i += 1,
            'i' | 'I' => {
                i += 1;
                let (next, value) = read_plain_float(chars, i);
                if let Some(value) = value {
                    props.first = Some(value);
                    i = next;
                }
            }
            'l' | 'L' => {
                i += 1;
                let (next, value) = read_plain_float(chars, i);
                if let Some(value) = value {
                    props.left = Some(value);
                    i = next;
                }
            }
            'r' | 'R' => {
                i += 1;
                let (next, value) = read_plain_float(chars, i);
                if let Some(value) = value {
                    props.right = Some(value);
                    i = next;
                }
            }
            'q' | 'Q' => {
                i += 1;
                if i < chars.len() {
                    props.align = Some(match chars[i] {
                        'c' | 'C' => MTextAlign::Center,
                        'r' | 'R' => MTextAlign::Right,
                        'j' | 'J' => MTextAlign::Justify,
                        'd' | 'D' => MTextAlign::Distribute,
                        _ => MTextAlign::Left,
                    });
                    if chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                }
            }
            't' | 'T' => {
                i += 1;
                while i < chars.len() && chars[i] != ';' {
                    let ch = chars[i];
                    if ch.is_ascii_alphabetic() && is_paragraph_key(ch) {
                        break;
                    }
                    i += 1;
                }
            }
            'x' | 'X' | '*' => i += 1,
            _ => i += 1,
        }
    }
    if i < chars.len() && chars[i] == ';' {
        i += 1;
    }
    (i, props)
}

fn is_paragraph_key(ch: char) -> bool {
    matches!(
        ch,
        'i' | 'I' | 'l' | 'L' | 'r' | 'R' | 'q' | 'Q' | 't' | 'T' | 'x' | 'X' | '*'
    )
}

fn read_plain_float(chars: &[char], start: usize) -> (usize, Option<f64>) {
    let mut i = start;
    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
        i += 1;
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            saw_digit = true;
            i += 1;
        } else if chars[i] == '.' && !saw_dot {
            saw_dot = true;
            i += 1;
        } else {
            break;
        }
    }
    if !saw_digit {
        return (start, None);
    }
    let parsed = chars[start..i]
        .iter()
        .collect::<String>()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite());
    (i, parsed)
}

fn read_font_spec(chars: &[char], start: usize) -> (usize, Option<FontSpec>) {
    let end = chars[start..]
        .iter()
        .position(|ch| *ch == ';')
        .map(|offset| start + offset)
        .unwrap_or(chars.len());
    let body: String = chars[start..end].iter().collect();
    let mut parts = body.split('|');
    let family = parts.next().unwrap_or("").trim().to_string();
    let mut bold = false;
    let mut italic = false;
    for part in parts {
        let part = part.trim();
        let mut letters = part.chars();
        let Some(key) = letters.next() else {
            continue;
        };
        let value: String = letters.collect();
        match key {
            'b' | 'B' => bold = value != "0",
            'i' | 'I' => italic = value != "0",
            _ => {}
        }
    }
    let next = if end < chars.len() { end + 1 } else { end };
    if body.is_empty() {
        return (next, None);
    }
    (
        next,
        Some(FontSpec {
            family,
            bold,
            italic,
        }),
    )
}

fn read_mtext_number(chars: &[char], mut i: usize) -> (usize, Option<(f64, bool)>) {
    let start = i;
    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
        i += 1;
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            saw_digit = true;
            i += 1;
        } else if chars[i] == '.' && !saw_dot {
            saw_dot = true;
            i += 1;
        } else {
            break;
        }
    }
    let relative = i < chars.len() && (chars[i] == 'x' || chars[i] == 'X');
    let number_end = i;
    if relative {
        i += 1;
    }
    let parsed = if saw_digit {
        chars[start..number_end]
            .iter()
            .collect::<String>()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    } else {
        None
    };
    while i < chars.len() && chars[i] != ';' {
        i += 1;
    }
    if i < chars.len() {
        i += 1;
    }
    (i, parsed.map(|value| (value, relative)))
}

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

    #[test]
    fn mtext_height_and_width_codes_change_the_run() {
        let absolute = mtext_paragraphs(r"\H4;Hi", 2.0);
        assert_eq!(absolute[0].runs[0].height, 4.0);
        assert_eq!(absolute[0].runs[0].text, "Hi");
        let relative = mtext_paragraphs(r"\H0.5x;Hi", 10.0);
        assert_eq!(relative[0].runs[0].height, 5.0);
        let wide = mtext_paragraphs(r"\W2;Hi", 2.0);
        assert_eq!(wide[0].runs[0].width_factor, 2.0);
        assert_eq!(wide[0].runs[0].height, 2.0);
        let scoped = mtext_paragraphs(r"{\H0.5x;A}B", 10.0);
        assert_eq!(scoped[0].runs[0].text, "A");
        assert_eq!(scoped[0].runs[0].height, 5.0);
        assert_eq!(scoped[0].runs[1].text, "B");
        assert_eq!(scoped[0].runs[1].height, 10.0);
    }

    #[test]
    fn mtext_paragraph_alignment_and_indents() {
        let right = mtext_paragraphs(r"\pxqr;Hi", 2.0);
        assert_eq!(right[0].align, Some(MTextAlign::Right));
        assert_eq!(right[0].runs[0].text, "Hi");
        let center = mtext_paragraphs(r"\pxqc;Hi", 2.0);
        assert_eq!(center[0].align, Some(MTextAlign::Center));
        let indented = mtext_paragraphs(r"\pi1.5,l2,r0.5;One\PTwo", 4.0);
        assert_eq!(indented[0].first_indent, 1.5);
        assert_eq!(indented[0].left_indent, 2.0);
        assert_eq!(indented[0].right_indent, 0.5);
        assert_eq!(indented[0].runs[0].text, "One");
        assert!(indented[1].align.is_none());
        assert_eq!(indented[1].first_indent, 0.0);
        assert_eq!(indented[1].runs[0].text, "Two");
    }

    #[test]
    fn mtext_font_code_is_scoped_by_braces() {
        let switched = mtext_paragraphs(r"\fArial|b1|i0;Hi", 2.0);
        assert_eq!(switched[0].runs[0].font, "Arial");
        assert!(switched[0].runs[0].bold);
        assert!(!switched[0].runs[0].italic);
        let scoped = mtext_paragraphs(r"{\fArial Narrow|b0|i1;A}B", 10.0);
        assert_eq!(scoped[0].runs[0].text, "A");
        assert_eq!(scoped[0].runs[0].font, "Arial Narrow");
        assert!(scoped[0].runs[0].italic);
        assert_eq!(scoped[0].runs[1].text, "B");
        assert!(scoped[0].runs[1].font.is_empty());
        assert!(!scoped[0].runs[1].italic);
    }
}
