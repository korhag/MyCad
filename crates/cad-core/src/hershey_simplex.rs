//! Public-domain Hershey Simplex stroke shapes.
//!
//! Glyphs are the Roman Simplex font drawn by Dr. A. V. Hershey and
//! distributed by James Hurt. Coordinates are Hershey units: y = 0 is
//! the baseline and y = 21 is the cap height. A pair of -1,-1 lifts the
//! pen. `advance` and every stroke are scaled by 1/21 so one unit of
//! `height` is the cap height.

const UNIT: f64 = 1.0 / 21.0;
const I_DOT_INDEX: usize = (b'i' - 32) as usize;
const J_DOT_INDEX: usize = (b'j' - 32) as usize;

struct RawGlyph {
    advance: i8,
    pts: &'static [i8],
}

#[rustfmt::skip]
const GLYPHS: [RawGlyph; 95] = [
    /* 32 ' ' */ RawGlyph { advance: 16, pts: &[] },
    /* 33 '!' */ RawGlyph { advance: 10, pts: &[5, 21, 5, 7, -1, -1, 5, 2, 4, 1, 5, 0, 6, 1, 5, 2] },
    /* 34 '"' */ RawGlyph { advance: 16, pts: &[4, 21, 4, 14, -1, -1, 12, 21, 12, 14] },
    /* 35 '#' */ RawGlyph { advance: 21, pts: &[11, 25, 4, -7, -1, -1, 17, 25, 10, -7, -1, -1, 4, 12, 18, 12, -1, -1, 3, 6, 17, 6] },
    /* 36 '$' */ RawGlyph { advance: 20, pts: &[8, 25, 8, -4, -1, -1, 12, 25, 12, -4, -1, -1, 17, 18, 15, 20, 12, 21, 8, 21, 5, 20, 3, 18, 3, 16, 4, 14, 5, 13, 7, 12, 13, 10, 15, 9, 16, 8, 17, 6, 17, 3, 15, 1, 12, 0, 8, 0, 5, 1, 3, 3] },
    /* 37 '%' */ RawGlyph { advance: 24, pts: &[21, 21, 3, 0, -1, -1, 8, 21, 10, 19, 10, 17, 9, 15, 7, 14, 5, 14, 3, 16, 3, 18, 4, 20, 6, 21, 8, 21, 10, 20, 13, 19, 16, 19, 19, 20, 21, 21, -1, -1, 17, 7, 15, 6, 14, 4, 14, 2, 16, 0, 18, 0, 20, 1, 21, 3, 21, 5, 19, 7, 17, 7] },
    /* 38 '&' */ RawGlyph { advance: 26, pts: &[23, 12, 23, 13, 22, 14, 21, 14, 20, 13, 19, 11, 17, 6, 15, 3, 13, 1, 11, 0, 7, 0, 5, 1, 4, 2, 3, 4, 3, 6, 4, 8, 5, 9, 12, 13, 13, 14, 14, 16, 14, 18, 13, 20, 11, 21, 9, 20, 8, 18, 8, 16, 9, 13, 11, 10, 16, 3, 18, 1, 20, 0, 22, 0, 23, 1, 23, 2] },
    /* 39 ''' */ RawGlyph { advance: 10, pts: &[5, 19, 4, 20, 5, 21, 6, 20, 6, 18, 5, 16, 4, 15] },
    /* 40 '(' */ RawGlyph { advance: 14, pts: &[11, 25, 9, 23, 7, 20, 5, 16, 4, 11, 4, 7, 5, 2, 7, -2, 9, -5, 11, -7] },
    /* 41 ')' */ RawGlyph { advance: 14, pts: &[3, 25, 5, 23, 7, 20, 9, 16, 10, 11, 10, 7, 9, 2, 7, -2, 5, -5, 3, -7] },
    /* 42 '?' */ RawGlyph { advance: 16, pts: &[8, 21, 8, 9, -1, -1, 3, 18, 13, 12, -1, -1, 13, 18, 3, 12] },
    /* 43 '+' */ RawGlyph { advance: 26, pts: &[13, 18, 13, 0, -1, -1, 4, 9, 22, 9] },
    /* 44 ',' */ RawGlyph { advance: 10, pts: &[6, 1, 5, 0, 4, 1, 5, 2, 6, 1, 6, -1, 5, -3, 4, -4] },
    /* 45 '-' */ RawGlyph { advance: 26, pts: &[4, 9, 22, 9] },
    /* 46 '.' */ RawGlyph { advance: 10, pts: &[5, 2, 4, 1, 5, 0, 6, 1, 5, 2] },
    /* 47 '?' */ RawGlyph { advance: 22, pts: &[20, 25, 2, -7] },
    /* 48 '0' */ RawGlyph { advance: 20, pts: &[9, 21, 6, 20, 4, 17, 3, 12, 3, 9, 4, 4, 6, 1, 9, 0, 11, 0, 14, 1, 16, 4, 17, 9, 17, 12, 16, 17, 14, 20, 11, 21, 9, 21] },
    /* 49 '1' */ RawGlyph { advance: 20, pts: &[6, 17, 8, 18, 11, 21, 11, 0] },
    /* 50 '2' */ RawGlyph { advance: 20, pts: &[4, 16, 4, 17, 5, 19, 6, 20, 8, 21, 12, 21, 14, 20, 15, 19, 16, 17, 16, 15, 15, 13, 13, 10, 3, 0, 17, 0] },
    /* 51 '3' */ RawGlyph { advance: 20, pts: &[5, 21, 16, 21, 10, 13, 13, 13, 15, 12, 16, 11, 17, 8, 17, 6, 16, 3, 14, 1, 11, 0, 8, 0, 5, 1, 4, 2, 3, 4] },
    /* 52 '4' */ RawGlyph { advance: 20, pts: &[13, 21, 3, 7, 18, 7, -1, -1, 13, 21, 13, 0] },
    /* 53 '5' */ RawGlyph { advance: 20, pts: &[15, 21, 5, 21, 4, 12, 5, 13, 8, 14, 11, 14, 14, 13, 16, 11, 17, 8, 17, 6, 16, 3, 14, 1, 11, 0, 8, 0, 5, 1, 4, 2, 3, 4] },
    /* 54 '6' */ RawGlyph { advance: 20, pts: &[16, 18, 15, 20, 12, 21, 10, 21, 7, 20, 5, 17, 4, 12, 4, 7, 5, 3, 7, 1, 10, 0, 11, 0, 14, 1, 16, 3, 17, 6, 17, 7, 16, 10, 14, 12, 11, 13, 10, 13, 7, 12, 5, 10, 4, 7] },
    /* 55 '7' */ RawGlyph { advance: 20, pts: &[17, 21, 7, 0, -1, -1, 3, 21, 17, 21] },
    /* 56 '8' */ RawGlyph { advance: 20, pts: &[8, 21, 5, 20, 4, 18, 4, 16, 5, 14, 7, 13, 11, 12, 14, 11, 16, 9, 17, 7, 17, 4, 16, 2, 15, 1, 12, 0, 8, 0, 5, 1, 4, 2, 3, 4, 3, 7, 4, 9, 6, 11, 9, 12, 13, 13, 15, 14, 16, 16, 16, 18, 15, 20, 12, 21, 8, 21] },
    /* 57 '9' */ RawGlyph { advance: 20, pts: &[16, 14, 15, 11, 13, 9, 10, 8, 9, 8, 6, 9, 4, 11, 3, 14, 3, 15, 4, 18, 6, 20, 9, 21, 10, 21, 13, 20, 15, 18, 16, 14, 16, 9, 15, 4, 13, 1, 10, 0, 8, 0, 5, 1, 4, 3] },
    /* 58 ':' */ RawGlyph { advance: 10, pts: &[5, 14, 4, 13, 5, 12, 6, 13, 5, 14, -1, -1, 5, 2, 4, 1, 5, 0, 6, 1, 5, 2] },
    /* 59 ';' */ RawGlyph { advance: 10, pts: &[5, 14, 4, 13, 5, 12, 6, 13, 5, 14, -1, -1, 6, 1, 5, 0, 4, 1, 5, 2, 6, 1, 6, -1, 5, -3, 4, -4] },
    /* 60 '<' */ RawGlyph { advance: 24, pts: &[20, 18, 4, 9, 20, 0] },
    /* 61 '=' */ RawGlyph { advance: 26, pts: &[4, 12, 22, 12, -1, -1, 4, 6, 22, 6] },
    /* 62 '>' */ RawGlyph { advance: 24, pts: &[4, 18, 20, 9, 4, 0] },
    /* 63 '?' */ RawGlyph { advance: 18, pts: &[3, 16, 3, 17, 4, 19, 5, 20, 7, 21, 11, 21, 13, 20, 14, 19, 15, 17, 15, 15, 14, 13, 13, 12, 9, 10, 9, 7, -1, -1, 9, 2, 8, 1, 9, 0, 10, 1, 9, 2] },
    /* 64 '@' */ RawGlyph { advance: 27, pts: &[18, 13, 17, 15, 15, 16, 12, 16, 10, 15, 9, 14, 8, 11, 8, 8, 9, 6, 11, 5, 14, 5, 16, 6, 17, 8, -1, -1, 12, 16, 10, 14, 9, 11, 9, 8, 10, 6, 11, 5, -1, -1, 18, 16, 17, 8, 17, 6, 19, 5, 21, 5, 23, 7, 24, 10, 24, 12, 23, 15, 22, 17, 20, 19, 18, 20, 15, 21, 12, 21, 9, 20, 7, 19, 5, 17, 4, 15, 3, 12, 3, 9, 4, 6, 5, 4, 7, 2, 9, 1, 12, 0, 15, 0, 18, 1, 20, 2, 21, 3, -1, -1, 19, 16, 18, 8, 18, 6, 19, 5] },
    /* 65 'A' */ RawGlyph { advance: 18, pts: &[9, 21, 1, 0, -1, -1, 9, 21, 17, 0, -1, -1, 4, 7, 14, 7] },
    /* 66 'B' */ RawGlyph { advance: 21, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 13, 21, 16, 20, 17, 19, 18, 17, 18, 15, 17, 13, 16, 12, 13, 11, -1, -1, 4, 11, 13, 11, 16, 10, 17, 9, 18, 7, 18, 4, 17, 2, 16, 1, 13, 0, 4, 0] },
    /* 67 'C' */ RawGlyph { advance: 21, pts: &[18, 16, 17, 18, 15, 20, 13, 21, 9, 21, 7, 20, 5, 18, 4, 16, 3, 13, 3, 8, 4, 5, 5, 3, 7, 1, 9, 0, 13, 0, 15, 1, 17, 3, 18, 5] },
    /* 68 'D' */ RawGlyph { advance: 21, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 11, 21, 14, 20, 16, 18, 17, 16, 18, 13, 18, 8, 17, 5, 16, 3, 14, 1, 11, 0, 4, 0] },
    /* 69 'E' */ RawGlyph { advance: 19, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 17, 21, -1, -1, 4, 11, 12, 11, -1, -1, 4, 0, 17, 0] },
    /* 70 'F' */ RawGlyph { advance: 18, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 17, 21, -1, -1, 4, 11, 12, 11] },
    /* 71 'G' */ RawGlyph { advance: 21, pts: &[18, 16, 17, 18, 15, 20, 13, 21, 9, 21, 7, 20, 5, 18, 4, 16, 3, 13, 3, 8, 4, 5, 5, 3, 7, 1, 9, 0, 13, 0, 15, 1, 17, 3, 18, 5, 18, 8, -1, -1, 13, 8, 18, 8] },
    /* 72 'H' */ RawGlyph { advance: 22, pts: &[4, 21, 4, 0, -1, -1, 18, 21, 18, 0, -1, -1, 4, 11, 18, 11] },
    /* 73 'I' */ RawGlyph { advance: 8, pts: &[4, 21, 4, 0] },
    /* 74 'J' */ RawGlyph { advance: 16, pts: &[12, 21, 12, 5, 11, 2, 10, 1, 8, 0, 6, 0, 4, 1, 3, 2, 2, 5, 2, 7] },
    /* 75 'K' */ RawGlyph { advance: 21, pts: &[4, 21, 4, 0, -1, -1, 18, 21, 4, 7, -1, -1, 9, 12, 18, 0] },
    /* 76 'L' */ RawGlyph { advance: 17, pts: &[4, 21, 4, 0, -1, -1, 4, 0, 16, 0] },
    /* 77 'M' */ RawGlyph { advance: 24, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 12, 0, -1, -1, 20, 21, 12, 0, -1, -1, 20, 21, 20, 0] },
    /* 78 'N' */ RawGlyph { advance: 22, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 18, 0, -1, -1, 18, 21, 18, 0] },
    /* 79 'O' */ RawGlyph { advance: 22, pts: &[9, 21, 7, 20, 5, 18, 4, 16, 3, 13, 3, 8, 4, 5, 5, 3, 7, 1, 9, 0, 13, 0, 15, 1, 17, 3, 18, 5, 19, 8, 19, 13, 18, 16, 17, 18, 15, 20, 13, 21, 9, 21] },
    /* 80 'P' */ RawGlyph { advance: 21, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 13, 21, 16, 20, 17, 19, 18, 17, 18, 14, 17, 12, 16, 11, 13, 10, 4, 10] },
    /* 81 'Q' */ RawGlyph { advance: 22, pts: &[9, 21, 7, 20, 5, 18, 4, 16, 3, 13, 3, 8, 4, 5, 5, 3, 7, 1, 9, 0, 13, 0, 15, 1, 17, 3, 18, 5, 19, 8, 19, 13, 18, 16, 17, 18, 15, 20, 13, 21, 9, 21, -1, -1, 12, 4, 18, -2] },
    /* 82 'R' */ RawGlyph { advance: 21, pts: &[4, 21, 4, 0, -1, -1, 4, 21, 13, 21, 16, 20, 17, 19, 18, 17, 18, 15, 17, 13, 16, 12, 13, 11, 4, 11, -1, -1, 11, 11, 18, 0] },
    /* 83 'S' */ RawGlyph { advance: 20, pts: &[17, 18, 15, 20, 12, 21, 8, 21, 5, 20, 3, 18, 3, 16, 4, 14, 5, 13, 7, 12, 13, 10, 15, 9, 16, 8, 17, 6, 17, 3, 15, 1, 12, 0, 8, 0, 5, 1, 3, 3] },
    /* 84 'T' */ RawGlyph { advance: 16, pts: &[8, 21, 8, 0, -1, -1, 1, 21, 15, 21] },
    /* 85 'U' */ RawGlyph { advance: 22, pts: &[4, 21, 4, 6, 5, 3, 7, 1, 10, 0, 12, 0, 15, 1, 17, 3, 18, 6, 18, 21] },
    /* 86 'V' */ RawGlyph { advance: 18, pts: &[1, 21, 9, 0, -1, -1, 17, 21, 9, 0] },
    /* 87 'W' */ RawGlyph { advance: 24, pts: &[2, 21, 7, 0, -1, -1, 12, 21, 7, 0, -1, -1, 12, 21, 17, 0, -1, -1, 22, 21, 17, 0] },
    /* 88 'X' */ RawGlyph { advance: 20, pts: &[3, 21, 17, 0, -1, -1, 17, 21, 3, 0] },
    /* 89 'Y' */ RawGlyph { advance: 18, pts: &[1, 21, 9, 11, 9, 0, -1, -1, 17, 21, 9, 11] },
    /* 90 'Z' */ RawGlyph { advance: 20, pts: &[17, 21, 3, 0, -1, -1, 3, 21, 17, 21, -1, -1, 3, 0, 17, 0] },
    /* 91 '[' */ RawGlyph { advance: 14, pts: &[4, 25, 4, -7, -1, -1, 5, 25, 5, -7, -1, -1, 4, 25, 11, 25, -1, -1, 4, -7, 11, -7] },
    /* 92 '?' */ RawGlyph { advance: 14, pts: &[0, 21, 14, -3] },
    /* 93 ']' */ RawGlyph { advance: 14, pts: &[9, 25, 9, -7, -1, -1, 10, 25, 10, -7, -1, -1, 3, 25, 10, 25, -1, -1, 3, -7, 10, -7] },
    /* 94 '^' */ RawGlyph { advance: 16, pts: &[6, 15, 8, 18, 10, 15, -1, -1, 3, 12, 8, 17, 13, 12, -1, -1, 8, 17, 8, 0] },
    /* 95 '_' */ RawGlyph { advance: 16, pts: &[0, -2, 16, -2] },
    /* 96 '`' */ RawGlyph { advance: 10, pts: &[6, 21, 5, 20, 4, 18, 4, 16, 5, 15, 6, 16, 5, 17] },
    /* 97 'a' */ RawGlyph { advance: 19, pts: &[15, 14, 15, 0, -1, -1, 15, 11, 13, 13, 11, 14, 8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3] },
    /* 98 'b' */ RawGlyph { advance: 19, pts: &[4, 21, 4, 0, -1, -1, 4, 11, 6, 13, 8, 14, 11, 14, 13, 13, 15, 11, 16, 8, 16, 6, 15, 3, 13, 1, 11, 0, 8, 0, 6, 1, 4, 3] },
    /* 99 'c' */ RawGlyph { advance: 18, pts: &[15, 11, 13, 13, 11, 14, 8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3] },
    /* 100 'd' */ RawGlyph { advance: 19, pts: &[15, 21, 15, 0, -1, -1, 15, 11, 13, 13, 11, 14, 8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3] },
    /* 101 'e' */ RawGlyph { advance: 18, pts: &[3, 8, 15, 8, 15, 10, 14, 12, 13, 13, 11, 14, 8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3] },
    /* 102 'f' */ RawGlyph { advance: 12, pts: &[10, 21, 8, 21, 6, 20, 5, 17, 5, 0, -1, -1, 2, 14, 9, 14] },
    /* 103 'g' */ RawGlyph { advance: 19, pts: &[15, 14, 15, -2, 14, -5, 13, -6, 11, -7, 8, -7, 6, -6, -1, -1, 15, 11, 13, 13, 11, 14, 8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3] },
    /* 104 'h' */ RawGlyph { advance: 19, pts: &[4, 21, 4, 0, -1, -1, 4, 10, 7, 13, 9, 14, 12, 14, 14, 13, 15, 10, 15, 0] },
    /* 105 'i' */ RawGlyph { advance: 8, pts: &[3, 21, 4, 20, 5, 21, 4, 22, 3, 21, -1, -1, 4, 14, 4, 0] },
    /* 106 'j' */ RawGlyph { advance: 10, pts: &[5, 21, 6, 20, 7, 21, 6, 22, 5, 21, -1, -1, 6, 14, 6, -3, 5, -6, 3, -7, 1, -7] },
    /* 107 'k' */ RawGlyph { advance: 17, pts: &[4, 21, 4, 0, -1, -1, 14, 14, 4, 4, -1, -1, 8, 8, 15, 0] },
    /* 108 'l' */ RawGlyph { advance: 8, pts: &[4, 21, 4, 0] },
    /* 109 'm' */ RawGlyph { advance: 30, pts: &[4, 14, 4, 0, -1, -1, 4, 10, 7, 13, 9, 14, 12, 14, 14, 13, 15, 10, 15, 0, -1, -1, 15, 10, 18, 13, 20, 14, 23, 14, 25, 13, 26, 10, 26, 0] },
    /* 110 'n' */ RawGlyph { advance: 19, pts: &[4, 14, 4, 0, -1, -1, 4, 10, 7, 13, 9, 14, 12, 14, 14, 13, 15, 10, 15, 0] },
    /* 111 'o' */ RawGlyph { advance: 19, pts: &[8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3, 16, 6, 16, 8, 15, 11, 13, 13, 11, 14, 8, 14] },
    /* 112 'p' */ RawGlyph { advance: 19, pts: &[4, 14, 4, -7, -1, -1, 4, 11, 6, 13, 8, 14, 11, 14, 13, 13, 15, 11, 16, 8, 16, 6, 15, 3, 13, 1, 11, 0, 8, 0, 6, 1, 4, 3] },
    /* 113 'q' */ RawGlyph { advance: 19, pts: &[15, 14, 15, -7, -1, -1, 15, 11, 13, 13, 11, 14, 8, 14, 6, 13, 4, 11, 3, 8, 3, 6, 4, 3, 6, 1, 8, 0, 11, 0, 13, 1, 15, 3] },
    /* 114 'r' */ RawGlyph { advance: 13, pts: &[4, 14, 4, 0, -1, -1, 4, 8, 5, 11, 7, 13, 9, 14, 12, 14] },
    /* 115 's' */ RawGlyph { advance: 17, pts: &[14, 11, 13, 13, 10, 14, 7, 14, 4, 13, 3, 11, 4, 9, 6, 8, 11, 7, 13, 6, 14, 4, 14, 3, 13, 1, 10, 0, 7, 0, 4, 1, 3, 3] },
    /* 116 't' */ RawGlyph { advance: 12, pts: &[5, 21, 5, 4, 6, 1, 8, 0, 10, 0, -1, -1, 2, 14, 9, 14] },
    /* 117 'u' */ RawGlyph { advance: 19, pts: &[4, 14, 4, 4, 5, 1, 7, 0, 10, 0, 12, 1, 15, 4, -1, -1, 15, 14, 15, 0] },
    /* 118 'v' */ RawGlyph { advance: 16, pts: &[2, 14, 8, 0, -1, -1, 14, 14, 8, 0] },
    /* 119 'w' */ RawGlyph { advance: 22, pts: &[3, 14, 7, 0, -1, -1, 11, 14, 7, 0, -1, -1, 11, 14, 15, 0, -1, -1, 19, 14, 15, 0] },
    /* 120 'x' */ RawGlyph { advance: 17, pts: &[3, 14, 14, 0, -1, -1, 14, 14, 3, 0] },
    /* 121 'y' */ RawGlyph { advance: 16, pts: &[2, 14, 8, 0, -1, -1, 14, 14, 8, 0, 6, -4, 4, -6, 2, -7, 1, -7] },
    /* 122 'z' */ RawGlyph { advance: 17, pts: &[14, 14, 3, 0, -1, -1, 3, 14, 14, 14, -1, -1, 3, 0, 14, 0] },
    /* 123 '{' */ RawGlyph { advance: 14, pts: &[9, 25, 7, 24, 6, 23, 5, 21, 5, 19, 6, 17, 7, 16, 8, 14, 8, 12, 6, 10, -1, -1, 7, 24, 6, 22, 6, 20, 7, 18, 8, 17, 9, 15, 9, 13, 8, 11, 4, 9, 8, 7, 9, 5, 9, 3, 8, 1, 7, 0, 6, -2, 6, -4, 7, -6, -1, -1, 6, 8, 8, 6, 8, 4, 7, 2, 6, 1, 5, -1, 5, -3, 6, -5, 7, -6, 9, -7] },
    /* 124 '|' */ RawGlyph { advance: 8, pts: &[4, 25, 4, -7] },
    /* 125 '}' */ RawGlyph { advance: 14, pts: &[5, 25, 7, 24, 8, 23, 9, 21, 9, 19, 8, 17, 7, 16, 6, 14, 6, 12, 8, 10, -1, -1, 7, 24, 8, 22, 8, 20, 7, 18, 6, 17, 5, 15, 5, 13, 6, 11, 10, 9, 6, 7, 5, 5, 5, 3, 6, 1, 7, 0, 8, -2, 8, -4, 7, -6, -1, -1, 8, 8, 6, 6, 6, 4, 7, 2, 8, 1, 9, -1, 9, -3, 8, -5, 7, -6, 5, -7] },
    /* 126 '~' */ RawGlyph { advance: 24, pts: &[3, 6, 3, 8, 4, 11, 6, 12, 8, 12, 10, 11, 14, 8, 16, 7, 18, 7, 20, 8, 21, 10, -1, -1, 3, 8, 4, 10, 6, 11, 8, 11, 10, 10, 14, 7, 16, 6, 18, 6, 20, 7, 21, 10, 21, 12] },
];

// ------------------------------------------------------------
// Function: advance / for_each_segment
// Purpose: Width and pen strokes for one character, including
//          Latin-1 and Turkish letters built from a base glyph
//          plus a diacritic.
// ------------------------------------------------------------
pub fn advance(ch: char) -> f64 {
    units_of(ch) as f64 * UNIT
}

pub fn for_each_segment(ch: char, mut sink: impl FnMut((f64, f64), (f64, f64))) {
    draw_char(ch, &mut |a, b| sink(a, b));
}

fn units_of(ch: char) -> i16 {
    match composed(ch) {
        Composed::Ascii(index) => i16::from(GLYPHS[index].advance),
        Composed::Mark { base, .. } => i16::from(GLYPHS[base].advance),
        Composed::Shifted {
            left,
            right,
            overlap,
        } => i16::from(GLYPHS[left].advance) + i16::from(GLYPHS[right].advance) - overlap,
        Composed::Custom { advance, .. } => advance,
    }
}

#[derive(Clone, Copy)]
enum Mark {
    Acute,
    Grave,
    Circumflex,
    Diaeresis,
    Tilde,
    Ring,
    Cedilla,
    Breve,
    Dot,
    Caron,
    Stroke,
    Eth,
    Thorn,
}

enum Composed {
    Ascii(usize),
    Mark {
        base: usize,
        marks: &'static [Mark],
    },
    Shifted {
        left: usize,
        right: usize,
        overlap: i16,
    },
    Custom {
        advance: i16,
        pts: &'static [i8],
    },
}

fn ascii_index(ch: char) -> Option<usize> {
    let code = ch as u32;
    if (32..127).contains(&code) {
        Some((code - 32) as usize)
    } else {
        None
    }
}

fn index_of(ch: char) -> usize {
    ascii_index(ch).unwrap_or(0)
}

fn composed(ch: char) -> Composed {
    if let Some(index) = ascii_index(ch) {
        return Composed::Ascii(index);
    }
    if let Some((base, marks)) = diacritic(ch) {
        return Composed::Mark {
            base: index_of(base),
            marks,
        };
    }
    match ch {
        '\u{00C6}' => Composed::Shifted {
            left: index_of('A'),
            right: index_of('E'),
            overlap: 6,
        },
        '\u{00E6}' => Composed::Shifted {
            left: index_of('a'),
            right: index_of('e'),
            overlap: 5,
        },
        '\u{00D8}' => Composed::Mark {
            base: index_of('O'),
            marks: &[Mark::Stroke],
        },
        '\u{00F8}' => Composed::Mark {
            base: index_of('o'),
            marks: &[Mark::Stroke],
        },
        '\u{00D0}' => Composed::Mark {
            base: index_of('D'),
            marks: &[Mark::Eth],
        },
        '\u{00F0}' => Composed::Mark {
            base: index_of('d'),
            marks: &[Mark::Eth],
        },
        '\u{00DE}' => Composed::Mark {
            base: index_of('P'),
            marks: &[Mark::Thorn],
        },
        '\u{00FE}' => Composed::Ascii(index_of('p')),
        '\u{0131}' => Composed::Custom {
            advance: 8,
            pts: DOTLESS_I,
        },
        '\u{00DF}' => Composed::Custom {
            advance: 20,
            pts: SHARP_S,
        },
        '\u{00B0}' => Composed::Custom {
            advance: 12,
            pts: DEGREE,
        },
        '\u{00B1}' => Composed::Custom {
            advance: 20,
            pts: PLUS_MINUS,
        },
        '\u{2300}' => Composed::Custom {
            advance: 20,
            pts: DIAMETER,
        },
        _ => Composed::Custom {
            advance: 16,
            pts: MISSING,
        },
    }
}

fn diacritic(ch: char) -> Option<(char, &'static [Mark])> {
    use Mark::*;
    Some(match ch {
        '\u{00C0}' => ('A', &[Grave]),
        '\u{00C1}' => ('A', &[Acute]),
        '\u{00C2}' => ('A', &[Circumflex]),
        '\u{00C3}' => ('A', &[Tilde]),
        '\u{00C4}' => ('A', &[Diaeresis]),
        '\u{00C5}' => ('A', &[Ring]),
        '\u{00C7}' => ('C', &[Cedilla]),
        '\u{00C8}' => ('E', &[Grave]),
        '\u{00C9}' => ('E', &[Acute]),
        '\u{00CA}' => ('E', &[Circumflex]),
        '\u{00CB}' => ('E', &[Diaeresis]),
        '\u{00CC}' => ('I', &[Grave]),
        '\u{00CD}' => ('I', &[Acute]),
        '\u{00CE}' => ('I', &[Circumflex]),
        '\u{00CF}' => ('I', &[Diaeresis]),
        '\u{00D1}' => ('N', &[Tilde]),
        '\u{00D2}' => ('O', &[Grave]),
        '\u{00D3}' => ('O', &[Acute]),
        '\u{00D4}' => ('O', &[Circumflex]),
        '\u{00D5}' => ('O', &[Tilde]),
        '\u{00D6}' => ('O', &[Diaeresis]),
        '\u{00D9}' => ('U', &[Grave]),
        '\u{00DA}' => ('U', &[Acute]),
        '\u{00DB}' => ('U', &[Circumflex]),
        '\u{00DC}' => ('U', &[Diaeresis]),
        '\u{00DD}' => ('Y', &[Acute]),
        '\u{00E0}' => ('a', &[Grave]),
        '\u{00E1}' => ('a', &[Acute]),
        '\u{00E2}' => ('a', &[Circumflex]),
        '\u{00E3}' => ('a', &[Tilde]),
        '\u{00E4}' => ('a', &[Diaeresis]),
        '\u{00E5}' => ('a', &[Ring]),
        '\u{00E7}' => ('c', &[Cedilla]),
        '\u{00E8}' => ('e', &[Grave]),
        '\u{00E9}' => ('e', &[Acute]),
        '\u{00EA}' => ('e', &[Circumflex]),
        '\u{00EB}' => ('e', &[Diaeresis]),
        '\u{00EC}' => ('i', &[Grave]),
        '\u{00ED}' => ('i', &[Acute]),
        '\u{00EE}' => ('i', &[Circumflex]),
        '\u{00EF}' => ('i', &[Diaeresis]),
        '\u{00F1}' => ('n', &[Tilde]),
        '\u{00F2}' => ('o', &[Grave]),
        '\u{00F3}' => ('o', &[Acute]),
        '\u{00F4}' => ('o', &[Circumflex]),
        '\u{00F5}' => ('o', &[Tilde]),
        '\u{00F6}' => ('o', &[Diaeresis]),
        '\u{00F9}' => ('u', &[Grave]),
        '\u{00FA}' => ('u', &[Acute]),
        '\u{00FB}' => ('u', &[Circumflex]),
        '\u{00FC}' => ('u', &[Diaeresis]),
        '\u{00FD}' => ('y', &[Acute]),
        '\u{00FF}' => ('y', &[Diaeresis]),
        '\u{011E}' => ('G', &[Breve]),
        '\u{011F}' => ('g', &[Breve]),
        '\u{0130}' => ('I', &[Dot]),
        '\u{015E}' => ('S', &[Cedilla]),
        '\u{015F}' => ('s', &[Cedilla]),
        '\u{010C}' => ('C', &[Caron]),
        '\u{010D}' => ('c', &[Caron]),
        '\u{0160}' => ('S', &[Caron]),
        '\u{0161}' => ('s', &[Caron]),
        '\u{017D}' => ('Z', &[Caron]),
        '\u{017E}' => ('z', &[Caron]),
        _ => return None,
    })
}

fn mark_above(mark: Mark) -> bool {
    !matches!(mark, Mark::Cedilla | Mark::Stroke | Mark::Eth | Mark::Thorn)
}

fn draw_char(ch: char, sink: &mut impl FnMut((f64, f64), (f64, f64))) {
    match composed(ch) {
        Composed::Ascii(index) => walk(&GLYPHS[index].pts, 0.0, 0.0, false, sink),
        Composed::Mark { base, marks } => {
            let glyph = &GLYPHS[base];
            // Only i and j keep a separate tittle. An accent replaces it;
            // a capital crossbar (all points near y = 21) must stay.
            let drop_dot =
                marks.iter().copied().any(mark_above) && matches!(base, I_DOT_INDEX | J_DOT_INDEX);
            walk(&glyph.pts, 0.0, 0.0, drop_dot, sink);
            let (min_x, max_x, min_y, max_y) = bounds(&glyph.pts, drop_dot);
            let center = (f64::from(min_x) + f64::from(max_x)) * 0.5;
            for mark in marks {
                draw_mark(*mark, center, min_x, max_x, min_y, max_y, sink);
            }
        }
        Composed::Shifted {
            left,
            right,
            overlap,
        } => {
            walk(&GLYPHS[left].pts, 0.0, 0.0, false, sink);
            let shift = f64::from(GLYPHS[left].advance) - f64::from(overlap);
            walk(&GLYPHS[right].pts, shift, 0.0, false, sink);
        }
        Composed::Custom { pts, .. } => walk(pts, 0.0, 0.0, false, sink),
    }
}

fn draw_mark(
    mark: Mark,
    center: f64,
    min_x: i8,
    max_x: i8,
    min_y: i8,
    max_y: i8,
    sink: &mut impl FnMut((f64, f64), (f64, f64)),
) {
    match mark {
        Mark::Stroke => {
            let line = [min_x, min_y, max_x, max_y];
            walk(&line, 0.0, 0.0, false, sink);
        }
        Mark::Eth => {
            let y = if max_y > 16 { 10 } else { 8 };
            let line = [min_x.saturating_sub(3), y, min_x.saturating_add(6), y];
            walk(&line, 0.0, 0.0, false, sink);
        }
        Mark::Thorn => {
            let line = [min_x, 0, min_x, -7];
            walk(&line, 0.0, 0.0, false, sink);
        }
        Mark::Cedilla => walk(mark_pts(mark), center, f64::from(min_y), false, sink),
        other => walk(mark_pts(other), center, f64::from(max_y) + 2.0, false, sink),
    }
}

fn mark_pts(mark: Mark) -> &'static [i8] {
    match mark {
        Mark::Acute => &[-2, 0, 3, 6],
        Mark::Grave => &[2, 0, -3, 6],
        Mark::Circumflex => &[-4, 0, 0, 5, 4, 0],
        Mark::Diaeresis => &[
            -5, 1, -3, 3, -4, 4, -6, 3, -5, 1, -1, -1, 1, 1, 3, 3, 2, 4, 0, 3, 1, 1,
        ],
        Mark::Tilde => &[-4, 1, -2, 4, 0, 2, 2, 1, 4, 4],
        Mark::Ring => &[0, 1, -3, 3, -2, 6, 1, 6, 2, 3, 0, 1],
        Mark::Cedilla => &[1, 0, -1, -2, -1, -4, 1, -5, 3, -3, 2, -1],
        Mark::Breve => &[-4, 4, -2, 1, 0, 0, 2, 1, 4, 4],
        Mark::Dot => &[-2, 1, -1, 3, 0, 4, 1, 3, 2, 1, -2, 1],
        Mark::Caron => &[-4, 5, 0, 0, 4, 5],
        Mark::Stroke | Mark::Eth | Mark::Thorn => &[],
    }
}

fn bounds(pts: &[i8], drop_high_dot: bool) -> (i8, i8, i8, i8) {
    let mut min_x = i8::MAX;
    let mut max_x = i8::MIN;
    let mut min_y = i8::MAX;
    let mut max_y = i8::MIN;
    for stroke in strokes_of(pts) {
        if drop_high_dot && is_high_dot(stroke) {
            continue;
        }
        for pair in stroke.chunks_exact(2) {
            min_x = min_x.min(pair[0]);
            max_x = max_x.max(pair[0]);
            min_y = min_y.min(pair[1]);
            max_y = max_y.max(pair[1]);
        }
    }
    if min_x > max_x {
        (0, 0, 0, 0)
    } else {
        (min_x, max_x, min_y, max_y)
    }
}

fn is_high_dot(stroke: &[i8]) -> bool {
    stroke.chunks_exact(2).all(|pair| pair[1] >= 18)
}

fn strokes_of(pts: &[i8]) -> impl Iterator<Item = &[i8]> {
    StrokeSplit { pts, index: 0 }
}

struct StrokeSplit<'a> {
    pts: &'a [i8],
    index: usize,
}

impl<'a> Iterator for StrokeSplit<'a> {
    type Item = &'a [i8];

    fn next(&mut self) -> Option<Self::Item> {
        let pts = self.pts;
        while self.index + 1 < pts.len() && pts[self.index] == -1 && pts[self.index + 1] == -1 {
            self.index += 2;
        }
        if self.index + 1 >= pts.len() {
            return None;
        }
        let start = self.index;
        while self.index + 1 < pts.len() && !(pts[self.index] == -1 && pts[self.index + 1] == -1) {
            self.index += 2;
        }
        let stroke = &pts[start..self.index];
        if self.index + 1 < pts.len() {
            self.index += 2;
        }
        Some(stroke)
    }
}

fn walk(
    pts: &[i8],
    x_add: f64,
    y_add: f64,
    drop_high_dot: bool,
    sink: &mut impl FnMut((f64, f64), (f64, f64)),
) {
    for stroke in strokes_of(pts) {
        if drop_high_dot && is_high_dot(stroke) {
            continue;
        }
        let mut prev: Option<(f64, f64)> = None;
        for pair in stroke.chunks_exact(2) {
            let point = (
                (f64::from(pair[0]) + x_add) * UNIT,
                (f64::from(pair[1]) + y_add) * UNIT,
            );
            if let Some(start) = prev {
                sink(start, point);
            }
            prev = Some(point);
        }
    }
}

const DOTLESS_I: &[i8] = &[4, 14, 4, 0];
const SHARP_S: &[i8] = &[
    14, 14, 10, 16, 6, 16, 3, 14, 3, 10, 6, 8, 12, 7, 15, 5, 16, 2, 14, 0, 8, 0, 4, 1, 3, 3,
];
const DEGREE: &[i8] = &[5, 14, 2, 16, 2, 19, 5, 21, 8, 19, 8, 16, 5, 14];
const PLUS_MINUS: &[i8] = &[3, 14, 17, 14, -1, -1, 10, 8, 10, 20, -1, -1, 3, 4, 17, 4];
const DIAMETER: &[i8] = &[
    10, 18, 6, 17, 4, 14, 3, 10, 4, 6, 6, 3, 10, 2, 14, 3, 16, 6, 17, 10, 16, 14, 14, 17, 10, 18,
    -1, -1, 4, 3, 16, 17,
];
const MISSING: &[i8] = &[2, 2, 12, 2, 12, 14, 2, 14, 2, 2];
