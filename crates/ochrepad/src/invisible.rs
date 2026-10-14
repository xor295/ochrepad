//! Characters that draw nothing on their own: control characters, format
//! characters (zero width, bidi controls, the BOM) and spaces other than
//! U+0020. The editor draws each as a tinted box with an abbreviation
//! (PLAN_v0.3.0 P3): control characters always, as Notepad++ does, and the
//! others with View > Show Symbol > Show Non-Printing Characters.
//!
//! Every one of them is its own grapheme (Unicode gives them the Control
//! grapheme break property) and takes at least one cell in
//! `core::position::grapheme_width`, so showing or hiding the boxes never
//! moves a column.

/// What to draw for a special character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Special {
    pub ch: char,
    /// A short name drawn in the box, such as `NUL` or `ZWSP`.
    pub abbrev: &'static str,
    /// The Unicode name, for the tooltip.
    pub name: &'static str,
    /// A C0 or C1 control character (or DEL), drawn even when non-printing
    /// characters are hidden.
    pub control: bool,
}

const C0: [(&str, &str); 32] = [
    ("NUL", "NULL"),
    ("SOH", "START OF HEADING"),
    ("STX", "START OF TEXT"),
    ("ETX", "END OF TEXT"),
    ("EOT", "END OF TRANSMISSION"),
    ("ENQ", "ENQUIRY"),
    ("ACK", "ACKNOWLEDGE"),
    ("BEL", "BELL"),
    ("BS", "BACKSPACE"),
    ("HT", "CHARACTER TABULATION"),
    ("LF", "LINE FEED"),
    ("VT", "LINE TABULATION"),
    ("FF", "FORM FEED"),
    ("CR", "CARRIAGE RETURN"),
    ("SO", "SHIFT OUT"),
    ("SI", "SHIFT IN"),
    ("DLE", "DATA LINK ESCAPE"),
    ("DC1", "DEVICE CONTROL ONE"),
    ("DC2", "DEVICE CONTROL TWO"),
    ("DC3", "DEVICE CONTROL THREE"),
    ("DC4", "DEVICE CONTROL FOUR"),
    ("NAK", "NEGATIVE ACKNOWLEDGE"),
    ("SYN", "SYNCHRONOUS IDLE"),
    ("ETB", "END OF TRANSMISSION BLOCK"),
    ("CAN", "CANCEL"),
    ("EM", "END OF MEDIUM"),
    ("SUB", "SUBSTITUTE"),
    ("ESC", "ESCAPE"),
    ("FS", "INFORMATION SEPARATOR FOUR"),
    ("GS", "INFORMATION SEPARATOR THREE"),
    ("RS", "INFORMATION SEPARATOR TWO"),
    ("US", "INFORMATION SEPARATOR ONE"),
];

const C1: [&str; 32] = [
    "PAD", "HOP", "BPH", "NBH", "IND", "NEL", "SSA", "ESA", "HTS", "HTJ", "VTS", "PLD", "PLU",
    "RI", "SS2", "SS3", "DCS", "PU1", "PU2", "STS", "CCH", "MW", "SPA", "EPA", "SOS", "SGC", "SCI",
    "CSI", "ST", "OSC", "PM", "APC",
];

/// Format characters and unusual spaces: (character, abbreviation, name).
const OTHERS: &[(char, &str, &str)] = &[
    ('\u{00A0}', "NBSP", "NO-BREAK SPACE"),
    ('\u{00AD}', "SHY", "SOFT HYPHEN"),
    ('\u{034F}', "CGJ", "COMBINING GRAPHEME JOINER"),
    ('\u{061C}', "ALM", "ARABIC LETTER MARK"),
    ('\u{115F}', "HCF", "HANGUL CHOSEONG FILLER"),
    ('\u{1160}', "HJF", "HANGUL JUNGSEONG FILLER"),
    ('\u{17B4}', "KIVAQ", "KHMER VOWEL INHERENT AQ"),
    ('\u{17B5}', "KIVAA", "KHMER VOWEL INHERENT AA"),
    ('\u{180E}', "MVS", "MONGOLIAN VOWEL SEPARATOR"),
    ('\u{2000}', "NQSP", "EN QUAD"),
    ('\u{2001}', "MQSP", "EM QUAD"),
    ('\u{2002}', "ENSP", "EN SPACE"),
    ('\u{2003}', "EMSP", "EM SPACE"),
    ('\u{2004}', "3MSP", "THREE-PER-EM SPACE"),
    ('\u{2005}', "4MSP", "FOUR-PER-EM SPACE"),
    ('\u{2006}', "6MSP", "SIX-PER-EM SPACE"),
    ('\u{2007}', "FSP", "FIGURE SPACE"),
    ('\u{2008}', "PSP", "PUNCTUATION SPACE"),
    ('\u{2009}', "THSP", "THIN SPACE"),
    ('\u{200A}', "HSP", "HAIR SPACE"),
    ('\u{200B}', "ZWSP", "ZERO WIDTH SPACE"),
    ('\u{200C}', "ZWNJ", "ZERO WIDTH NON-JOINER"),
    ('\u{200D}', "ZWJ", "ZERO WIDTH JOINER"),
    ('\u{200E}', "LRM", "LEFT-TO-RIGHT MARK"),
    ('\u{200F}', "RLM", "RIGHT-TO-LEFT MARK"),
    ('\u{2028}', "LSEP", "LINE SEPARATOR"),
    ('\u{2029}', "PSEP", "PARAGRAPH SEPARATOR"),
    ('\u{202A}', "LRE", "LEFT-TO-RIGHT EMBEDDING"),
    ('\u{202B}', "RLE", "RIGHT-TO-LEFT EMBEDDING"),
    ('\u{202C}', "PDF", "POP DIRECTIONAL FORMATTING"),
    ('\u{202D}', "LRO", "LEFT-TO-RIGHT OVERRIDE"),
    ('\u{202E}', "RLO", "RIGHT-TO-LEFT OVERRIDE"),
    ('\u{202F}', "NNBSP", "NARROW NO-BREAK SPACE"),
    ('\u{205F}', "MMSP", "MEDIUM MATHEMATICAL SPACE"),
    ('\u{2060}', "WJ", "WORD JOINER"),
    ('\u{2061}', "FA", "FUNCTION APPLICATION"),
    ('\u{2062}', "IT", "INVISIBLE TIMES"),
    ('\u{2063}', "IS", "INVISIBLE SEPARATOR"),
    ('\u{2064}', "IP", "INVISIBLE PLUS"),
    ('\u{2066}', "LRI", "LEFT-TO-RIGHT ISOLATE"),
    ('\u{2067}', "RLI", "RIGHT-TO-LEFT ISOLATE"),
    ('\u{2068}', "FSI", "FIRST STRONG ISOLATE"),
    ('\u{2069}', "PDI", "POP DIRECTIONAL ISOLATE"),
    ('\u{206A}', "ISS", "INHIBIT SYMMETRIC SWAPPING"),
    ('\u{206B}', "ASS", "ACTIVATE SYMMETRIC SWAPPING"),
    ('\u{206C}', "IAFS", "INHIBIT ARABIC FORM SHAPING"),
    ('\u{206D}', "AAFS", "ACTIVATE ARABIC FORM SHAPING"),
    ('\u{206E}', "NADS", "NATIONAL DIGIT SHAPES"),
    ('\u{206F}', "NODS", "NOMINAL DIGIT SHAPES"),
    ('\u{3000}', "IDSP", "IDEOGRAPHIC SPACE"),
    ('\u{3164}', "HF", "HANGUL FILLER"),
    (
        '\u{FEFF}',
        "BOM",
        "ZERO WIDTH NO-BREAK SPACE (BYTE ORDER MARK)",
    ),
    ('\u{FFA0}', "HWHF", "HALFWIDTH HANGUL FILLER"),
    ('\u{FFF9}', "IAA", "INTERLINEAR ANNOTATION ANCHOR"),
    ('\u{FFFA}', "IAS", "INTERLINEAR ANNOTATION SEPARATOR"),
    ('\u{FFFB}', "IAT", "INTERLINEAR ANNOTATION TERMINATOR"),
    ('\u{E0001}', "LANG", "LANGUAGE TAG"),
];

/// The special character a grapheme starts with, if any. Tab and the
/// ordinary space are not special (Show Space and Tab draws those).
pub fn special(g: &str) -> Option<Special> {
    let ch = g.chars().next()?;
    let c = ch as u32;
    if c < 0x20 && ch != '\t' {
        let (abbrev, name) = C0[c as usize];
        return Some(Special {
            ch,
            abbrev,
            name,
            control: true,
        });
    }
    if ch == '\u{7F}' {
        return Some(Special {
            ch,
            abbrev: "DEL",
            name: "DELETE",
            control: true,
        });
    }
    if (0x80..0xA0).contains(&c) {
        return Some(Special {
            ch,
            abbrev: C1[(c - 0x80) as usize],
            name: "C1 CONTROL CHARACTER",
            control: true,
        });
    }
    if c < 0xA0 {
        return None;
    }
    if let Ok(i) = OTHERS.binary_search_by_key(&ch, |o| o.0) {
        let (_, abbrev, name) = OTHERS[i];
        return Some(Special {
            ch,
            abbrev,
            name,
            control: false,
        });
    }
    let (abbrev, name) = match c {
        0xFE00..=0xFE0F | 0xE0100..=0xE01EF => ("VS", "VARIATION SELECTOR"),
        0xE0020..=0xE007F => ("TAG", "TAG CHARACTER"),
        _ => return None,
    };
    Some(Special {
        ch,
        abbrev,
        name,
        control: false,
    })
}

/// Whether the grapheme starting with byte `b` of a line can be special
/// without looking further: a quick filter for ASCII text.
pub fn maybe_special(b: u8) -> bool {
    !(0x20..0x7F).contains(&b)
}

/// The tooltip for a special character: its code point and name.
pub fn describe(s: &Special) -> String {
    format!("U+{:04X} {} ({})", s.ch as u32, s.name, s.abbrev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_for_binary_search() {
        assert!(OTHERS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn controls_and_invisible_characters_are_named() {
        let s = special("\0").unwrap();
        assert_eq!((s.abbrev, s.control), ("NUL", true));
        assert_eq!(special("\u{1b}").unwrap().abbrev, "ESC");
        assert_eq!(special("\u{7f}").unwrap().abbrev, "DEL");
        assert_eq!(special("\u{85}").unwrap().abbrev, "NEL");
        let z = special("\u{200B}").unwrap();
        assert_eq!((z.abbrev, z.control), ("ZWSP", false));
        assert_eq!(special("\u{FEFF}").unwrap().abbrev, "BOM");
        assert_eq!(special("\u{202E}").unwrap().abbrev, "RLO");
        assert_eq!(special("\u{A0}").unwrap().abbrev, "NBSP");
        assert_eq!(special("\u{FE0F}").unwrap().abbrev, "VS");
        assert_eq!(describe(&z), "U+200B ZERO WIDTH SPACE (ZWSP)");
    }

    #[test]
    fn ordinary_text_tabs_and_spaces_are_not_special() {
        for g in ["a", " ", "\t", "é", "漢", "👨‍👩‍👧", "\u{FFFD}", ""] {
            assert_eq!(special(g), None, "{g:?}");
        }
    }

    #[test]
    fn every_special_character_is_its_own_grapheme() {
        use unicode_segmentation::UnicodeSegmentation;
        // Text before it must not absorb it, or its box would be lost
        // inside another grapheme (combining marks and joiners excepted:
        // they only stand alone at the start of a line).
        let joiners = ['\u{034F}', '\u{200C}', '\u{200D}'];
        for &(ch, abbrev, _) in OTHERS {
            if joiners.contains(&ch) || abbrev == "KIVAQ" || abbrev == "KIVAA" {
                continue;
            }
            let text = format!("a{ch}b");
            let count = text.graphemes(true).count();
            assert_eq!(count, 3, "{abbrev} joins its neighbours");
        }
    }
}
