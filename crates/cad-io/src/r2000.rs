//! AutoCAD 2000 (AC1015) DXF string encoding.
//!
//! R2000 stores Unicode as `\U+XXXX` control sequences. UTF-8 in the file
//! itself is only valid from AutoCAD 2007 onward.

use std::fmt::Write as _;

pub const MTEXT_CHUNK: usize = 250;

// ------------------------------------------------------------
// Function: encode_dxf_r2000
// Purpose: Map every non-ASCII character to a `\U+XXXX` sequence.
// ------------------------------------------------------------
pub fn encode_dxf_r2000(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if is_plain_ascii(ch) {
            out.push(ch);
        } else {
            append_u_plus(&mut out, ch);
        }
    }
    out
}

// ------------------------------------------------------------
// Function: decode_dxf_r2000
// Purpose: Inverse of encode_dxf_r2000. Leaves already-decoded Unicode.
// ------------------------------------------------------------
pub fn decode_dxf_r2000(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        if let Some((ch, consumed)) = parse_u_plus(&chars, index) {
            out.push(ch);
            index += consumed;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

// ------------------------------------------------------------
// Function: mtext_group_chunks
// Purpose: Group 3 for every 250-byte piece except the last (group 1).
//          Never splits a `\U+XXXX` escape.
// ------------------------------------------------------------
pub fn mtext_group_chunks(encoded: &str) -> Vec<(i16, &str)> {
    if encoded.len() <= MTEXT_CHUNK {
        return vec![(1, encoded)];
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < encoded.len() {
        let remaining = encoded.len() - start;
        if remaining <= MTEXT_CHUNK {
            ranges.push((start, encoded.len()));
            break;
        }
        let mut end = start + MTEXT_CHUNK;
        end = retreat_from_escape(encoded, start, end);
        if end <= start {
            end = (start + MTEXT_CHUNK).min(encoded.len());
        }
        ranges.push((start, end));
        start = end;
    }
    let last = ranges.len() - 1;
    ranges
        .into_iter()
        .enumerate()
        .map(|(i, (a, b))| (if i == last { 1 } else { 3 }, &encoded[a..b]))
        .collect()
}

fn is_plain_ascii(ch: char) -> bool {
    let code = ch as u32;
    (0x20..=0x7E).contains(&code)
}

fn append_u_plus(out: &mut String, ch: char) {
    let code = ch as u32;
    if code <= 0xFFFF {
        let _ = write!(out, "\\U+{code:04X}");
        return;
    }
    let extra = code - 0x10000;
    let hi = 0xD800 + (extra >> 10);
    let lo = 0xDC00 + (extra & 0x3FF);
    let _ = write!(out, "\\U+{hi:04X}\\U+{lo:04X}");
}

fn parse_u_plus(chars: &[char], index: usize) -> Option<(char, usize)> {
    if index + 6 >= chars.len()
        || chars[index] != '\\'
        || chars[index + 1] != 'U'
        || chars[index + 2] != '+'
    {
        return None;
    }
    let hex: String = chars[index + 3..index + 7].iter().collect();
    let code = u32::from_str_radix(&hex, 16).ok()?;
    if (0xD800..=0xDBFF).contains(&code) && index + 13 < chars.len() {
        if chars[index + 7] == '\\' && chars[index + 8] == 'U' && chars[index + 9] == '+' {
            let hex_lo: String = chars[index + 10..index + 14].iter().collect();
            if let Ok(lo) = u32::from_str_radix(&hex_lo, 16) {
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let cp = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                    return char::from_u32(cp).map(|ch| (ch, 14));
                }
            }
        }
    }
    char::from_u32(code).map(|ch| (ch, 7))
}

fn retreat_from_escape(encoded: &str, start: usize, end: usize) -> usize {
    let bytes = encoded.as_bytes();
    let lookback = 6.min(end.saturating_sub(start));
    for delta in 0..=lookback {
        let index = end - delta;
        if index < start || index + 2 >= bytes.len() {
            continue;
        }
        if bytes[index] == b'\\' && bytes[index + 1] == b'U' && bytes[index + 2] == b'+' {
            let escape_end = index + 7;
            if escape_end > end {
                return index;
            }
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turkish_letters_become_u_plus_escapes() {
        let encoded = encode_dxf_r2000("Ölçü Şase Çıkış İstanbul ğ ş ı");
        assert!(encoded.is_ascii());
        assert!(encoded.contains("\\U+00D6"));
        assert!(encoded.contains("\\U+015E"));
        assert!(encoded.contains("\\U+0130"));
        assert!(encoded.contains("\\U+011F"));
        assert!(encoded.contains("\\U+0131"));
        assert_eq!(decode_dxf_r2000(&encoded), "Ölçü Şase Çıkış İstanbul ğ ş ı");
    }

    #[test]
    fn ascii_is_unchanged() {
        assert_eq!(encode_dxf_r2000("Pump-Layout"), "Pump-Layout");
        assert_eq!(decode_dxf_r2000("Pump-Layout"), "Pump-Layout");
    }

    #[test]
    fn mtext_chunks_put_group_1_last() {
        let text = "A".repeat(520);
        let chunks = mtext_group_chunks(&text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].0, 3);
        assert_eq!(chunks[1].0, 3);
        assert_eq!(chunks[2].0, 1);
        assert_eq!(chunks[0].1.len(), 250);
        assert_eq!(chunks[1].1.len(), 250);
        assert_eq!(chunks[2].1.len(), 20);
        assert_eq!(
            chunks.iter().map(|(_, part)| *part).collect::<String>(),
            text
        );
    }

    #[test]
    fn mtext_chunks_do_not_split_a_u_plus_escape() {
        let mut text = "A".repeat(247);
        text.push_str("\\U+015E");
        text.push_str(&"B".repeat(10));
        let chunks = mtext_group_chunks(&text);
        for (_, chunk) in &chunks {
            assert!(
                !chunk.ends_with('\\')
                    && !chunk.ends_with("\\U")
                    && !chunk.ends_with("\\U+")
                    && !chunk.contains("\\U+015")
                    || chunk.contains("\\U+015E"),
                "chunk split an escape: {chunk:?}"
            );
        }
        assert_eq!(
            chunks.iter().map(|(_, part)| *part).collect::<String>(),
            text
        );
        assert_eq!(*chunks.last().unwrap(), (1, chunks.last().unwrap().1));
    }
}
