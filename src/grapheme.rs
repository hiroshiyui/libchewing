//! Grouping Unicode codepoints into user perceived characters.
//!
//! Chewing uses one user perceived character as the smallest unit of the
//! pre-edit buffer. Most Chinese characters are a single Unicode codepoint, but
//! many characters people want to type are not:
//!
//! * Ideographic variation sequences append a variation selector to pick a
//!   specific glyph, e.g. 邊󠄀 is U+908A U+E0100.
//! * Emoji are often built from several codepoints joined by ZWJ, or refined by
//!   an emoji modifier, e.g. 👨‍👩‍👧 is U+1F468 U+200D U+1F469 U+200D U+1F467.
//! * Regional indicator pairs form flags, e.g. 🇹🇼 is U+1F1F9 U+1F1FC.
//! * Latin letters can be followed by combining marks, e.g. é can be written as
//!   U+0065 U+0301.
//!
//! [`graphemes`] splits a string into such units. It implements a deliberately
//! small subset of the extended grapheme cluster rules of [UAX #29][uax29]:
//! only variation selectors, combining marks from the common combining blocks,
//! emoji modifiers, ZWJ sequences, tag sequences, and regional indicator pairs
//! are joined to the preceding codepoint. Hangul syllable composition, Indic
//! conjunct clusters, and the full Unicode character property tables are out of
//! scope, because Chewing only needs to keep Chinese characters, symbols, and
//! emoji together. Keeping the rules in tree avoids depending on a Unicode
//! table that has to be updated for every Unicode release.
//!
//! [uax29]: https://www.unicode.org/reports/tr29/

use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::{Hash, Hasher},
    ops::Deref,
    str::from_utf8,
};

/// Zero width joiner.
const ZWJ: char = '\u{200D}';

/// Whether the codepoint continues the grapheme it follows.
fn is_extending(ch: char) -> bool {
    matches!(
        ch as u32,
        // Combining Diacritical Marks
        0x0300..=0x036F
        // Combining Diacritical Marks Extended
        | 0x1AB0..=0x1AFF
        // Combining Diacritical Marks Supplement
        | 0x1DC0..=0x1DFF
        // Zero width joiner
        | 0x200D
        // Combining Diacritical Marks for Symbols, includes the enclosing
        // keycap U+20E3
        | 0x20D0..=0x20F0
        // CJK Symbols and Punctuation combining marks
        | 0x302A..=0x302F
        // Combining Katakana-Hiragana voiced sound marks
        | 0x3099..=0x309A
        // Variation Selectors VS1 to VS16
        | 0xFE00..=0xFE0F
        // Combining Half Marks
        | 0xFE20..=0xFE2F
        // Emoji modifiers, the skin tones
        | 0x1F3FB..=0x1F3FF
        // Tags, used by emoji tag sequences such as the flag of England
        | 0xE0020..=0xE007F
        // Variation Selectors Supplement VS17 to VS256, used by ideographic
        // variation sequences
        | 0xE0100..=0xE01EF
    )
}

/// Whether the codepoint is a regional indicator, two of which form a flag.
fn is_regional_indicator(ch: char) -> bool {
    matches!(ch as u32, 0x1F1E6..=0x1F1FF)
}

/// Splits a string into user perceived characters.
///
/// The iterator never yields an empty string and never yields a slice longer
/// than [`Grapheme::CAPACITY`] bytes, so every item can be stored in a
/// [`Grapheme`]. A cluster longer than the capacity, which no assigned
/// character needs, is split at a codepoint boundary instead of being
/// truncated.
///
/// # Examples
///
/// ```
/// use chewing::grapheme::graphemes;
///
/// let chars: Vec<_> = graphemes("邊\u{E0100}好").collect();
///
/// assert_eq!(vec!["邊\u{E0100}", "好"], chars);
/// ```
pub fn graphemes(str: &str) -> Graphemes<'_> {
    Graphemes { rest: str }
}

/// An iterator over the user perceived characters of a string.
///
/// This struct is created by the [`graphemes`] function.
#[derive(Debug, Clone)]
pub struct Graphemes<'a> {
    rest: &'a str,
}

impl<'a> Graphemes<'a> {
    /// Returns the part of the string that has not been split yet.
    pub fn as_str(&self) -> &'a str {
        self.rest
    }
}

impl<'a> Iterator for Graphemes<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let mut chars = self.rest.chars();
        let base = chars.next()?;
        let mut end = base.len_utf8();

        if is_regional_indicator(base) {
            // A flag is a pair of regional indicators. A third one starts a new
            // flag instead of joining this one.
            if let Some(ch) = chars.next()
                && is_regional_indicator(ch)
            {
                end += ch.len_utf8();
            }
        } else {
            // The codepoint after a ZWJ always joins, whatever it is.
            let mut after_zwj = false;
            for ch in chars {
                if !after_zwj && !is_extending(ch) {
                    break;
                }
                if end + ch.len_utf8() > Grapheme::CAPACITY {
                    break;
                }
                end += ch.len_utf8();
                after_zwj = ch == ZWJ;
            }
        }

        let (grapheme, rest) = self.rest.split_at(end);
        self.rest = rest;
        Some(grapheme)
    }
}

/// A user perceived character made of one or more Unicode codepoints.
///
/// `Grapheme` stores the codepoints inline so that it stays [`Copy`] and needs
/// no allocation, at the cost of a fixed [`CAPACITY`][Grapheme::CAPACITY].
/// Build one from a [`char`], or from a string slice produced by
/// [`graphemes`].
///
/// # Examples
///
/// ```
/// use chewing::grapheme::Grapheme;
///
/// let ch = Grapheme::try_from("邊\u{E0100}")?;
///
/// assert_eq!("邊\u{E0100}", ch.as_str());
/// # Ok::<(), chewing::grapheme::GraphemeError>(())
/// ```
#[derive(Clone, Copy)]
pub struct Grapheme {
    len: u8,
    buf: [u8; Grapheme::CAPACITY],
}

impl Grapheme {
    /// The largest number of UTF-8 bytes a `Grapheme` can hold.
    ///
    /// This is large enough for every emoji sequence and ideographic variation
    /// sequence in Unicode.
    pub const CAPACITY: usize = 63;

    /// Returns the codepoints as a string slice.
    pub fn as_str(&self) -> &str {
        // The bytes always come from a &str, so they are always valid UTF-8.
        from_utf8(&self.buf[..self.len as usize]).unwrap_or_default()
    }
}

impl Debug for Grapheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self.as_str(), f)
    }
}

impl Display for Grapheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.as_str(), f)
    }
}

impl Deref for Grapheme {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Grapheme {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for Grapheme {
    fn eq(&self, other: &Grapheme) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Grapheme {}

impl PartialEq<str> for Grapheme {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Grapheme {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Grapheme> for str {
    fn eq(&self, other: &Grapheme) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Grapheme> for &str {
    fn eq(&self, other: &Grapheme) -> bool {
        *self == other.as_str()
    }
}

impl PartialOrd for Grapheme {
    fn partial_cmp(&self, other: &Grapheme) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Grapheme {
    fn cmp(&self, other: &Grapheme) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Grapheme {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl From<char> for Grapheme {
    fn from(value: char) -> Grapheme {
        let mut buf = [0; Grapheme::CAPACITY];
        let len = value.encode_utf8(&mut buf).len();
        Grapheme {
            len: len as u8,
            buf,
        }
    }
}

impl TryFrom<&str> for Grapheme {
    type Error = GraphemeError;

    fn try_from(value: &str) -> Result<Grapheme, GraphemeError> {
        if value.is_empty() {
            return Err(GraphemeError { len: 0 });
        }
        if value.len() > Grapheme::CAPACITY {
            return Err(GraphemeError { len: value.len() });
        }
        let mut buf = [0; Grapheme::CAPACITY];
        buf[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Grapheme {
            len: value.len() as u8,
            buf,
        })
    }
}

/// The error returned when a string cannot be stored in a [`Grapheme`].
///
/// A `Grapheme` must not be empty and must not be longer than
/// [`Grapheme::CAPACITY`] bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphemeError {
    len: usize,
}

impl Display for GraphemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.len == 0 {
            write!(f, "a grapheme cannot be empty")
        } else {
            write!(
                f,
                "a grapheme cannot be longer than {} bytes, got {}",
                Grapheme::CAPACITY,
                self.len
            )
        }
    }
}

impl Error for GraphemeError {}

#[cfg(test)]
mod tests {
    use super::{Grapheme, GraphemeError, graphemes};

    fn split(str: &str) -> Vec<&str> {
        graphemes(str).collect()
    }

    #[test]
    fn split_single_codepoint_characters() {
        assert_eq!(vec!["酷", "音", "，", "a"], split("酷音，a"));
    }

    #[test]
    fn split_empty_string() {
        assert!(split("").is_empty());
    }

    #[test]
    fn split_supplementary_plane_characters() {
        // U+20B9F is a single codepoint outside the BMP
        assert_eq!(vec!["\u{20B9F}", "好"], split("\u{20B9F}好"));
    }

    #[test]
    fn split_ideographic_variation_sequence() {
        assert_eq!(
            vec!["邊\u{E0100}", "邊\u{E0101}", "邊"],
            split("邊\u{E0100}邊\u{E0101}邊")
        );
    }

    #[test]
    fn split_variation_selector_16() {
        assert_eq!(vec!["☺\u{FE0F}", "☺"], split("☺\u{FE0F}☺"));
    }

    #[test]
    fn split_combining_mark() {
        assert_eq!(vec!["e\u{301}", "e"], split("e\u{301}e"));
    }

    #[test]
    fn split_zwj_sequence() {
        assert_eq!(
            vec!["👨\u{200D}👩\u{200D}👧", "好"],
            split("👨\u{200D}👩\u{200D}👧好")
        );
    }

    #[test]
    fn split_emoji_modifier() {
        assert_eq!(vec!["👍\u{1F3FB}", "👍"], split("👍\u{1F3FB}👍"));
    }

    #[test]
    fn split_keycap_sequence() {
        assert_eq!(vec!["1\u{FE0F}\u{20E3}", "2"], split("1\u{FE0F}\u{20E3}2"));
    }

    #[test]
    fn split_regional_indicator_pairs() {
        assert_eq!(vec!["🇹🇼", "🇯🇵", "🇦"], split("🇹🇼🇯🇵🇦"));
    }

    #[test]
    fn split_tag_sequence() {
        // The flag of England, a black flag followed by tag characters
        let england = "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}";
        assert_eq!(vec![england, "好"], split(&format!("{england}好")));
    }

    #[test]
    fn split_leading_combining_mark() {
        // A combining mark with nothing to combine with stands on its own
        assert_eq!(vec!["\u{301}", "e"], split("\u{301}e"));
    }

    #[test]
    fn split_cluster_longer_than_capacity_at_codepoint_boundary() {
        let long: String = std::iter::once('a')
            .chain(std::iter::repeat_n('\u{301}', 40))
            .collect();
        let parts = split(&long);

        assert!(parts.len() > 1, "the cluster should be split");
        assert!(parts.iter().all(|part| part.len() <= Grapheme::CAPACITY));
        assert_eq!(long, parts.concat(), "no codepoint should be lost");
    }

    #[test]
    fn split_cluster_exactly_at_capacity_stays_whole() {
        // 1 byte base plus 31 two byte combining marks is exactly the capacity
        let exact: String = std::iter::once('a')
            .chain(std::iter::repeat_n('\u{301}', 31))
            .collect();
        assert_eq!(Grapheme::CAPACITY, exact.len());

        assert_eq!(vec![exact.as_str()], split(&exact));
        assert!(Grapheme::try_from(exact.as_str()).is_ok());

        // One more mark does not fit and starts a new grapheme
        let over = format!("{exact}\u{301}");
        assert_eq!(vec![exact.as_str(), "\u{301}"], split(&over));
        assert!(Grapheme::try_from(over.as_str()).is_err());
    }

    #[test]
    fn split_trailing_zero_width_joiner() {
        // A ZWJ with nothing after it stays with the character it follows
        assert_eq!(vec!["\u{1F468}\u{200D}"], split("\u{1F468}\u{200D}"));
    }

    #[test]
    fn split_zwj_chain_cut_by_capacity() {
        // 4 + 7 * 9 = 67 bytes, so the chain cannot stay in one grapheme
        let chain = format!("\u{1F468}{}", "\u{200D}\u{1F468}".repeat(9));
        let parts = split(&chain);

        assert_eq!(chain, parts.concat(), "no codepoint should be lost");
        assert!(parts.iter().all(|part| part.len() <= Grapheme::CAPACITY));
        assert!(parts.len() > 1, "the chain should be split");
    }

    #[test]
    fn split_leading_zero_width_joiner() {
        // A ZWJ with nothing before it does not swallow the next character
        assert_eq!(vec!["\u{200D}", "a"], split("\u{200D}a"));
    }

    #[test]
    fn every_split_fits_in_a_grapheme() {
        let text = "酷音邊\u{E0100}👨\u{200D}👩\u{200D}👧🇹🇼1\u{FE0F}\u{20E3}";
        for part in split(text) {
            assert!(Grapheme::try_from(part).is_ok(), "{part} should fit");
        }
    }

    #[test]
    fn grapheme_from_char() {
        assert_eq!("酷", Grapheme::from('酷').as_str());
        assert_eq!("\u{20B9F}", Grapheme::from('\u{20B9F}').as_str());
    }

    #[test]
    fn grapheme_rejects_empty_and_too_long_strings() {
        assert!(Grapheme::try_from("").is_err());
        assert!(Grapheme::try_from("好".repeat(30).as_str()).is_err());
    }

    #[test]
    fn grapheme_error_is_readable() {
        let empty = Grapheme::try_from("").unwrap_err();
        assert_eq!("a grapheme cannot be empty", empty.to_string());

        let too_long = Grapheme::try_from("好".repeat(30).as_str()).unwrap_err();
        assert_eq!(
            format!("a grapheme cannot be longer than 63 bytes, got 90"),
            too_long.to_string()
        );
        assert_ne!(GraphemeError { len: 0 }, too_long);
    }

    #[test]
    fn grapheme_compares_by_text() {
        assert_eq!(Grapheme::from('好'), Grapheme::try_from("好").unwrap());
        assert_ne!(
            Grapheme::from('好'),
            Grapheme::try_from("好\u{E0100}").unwrap()
        );
        assert!(Grapheme::from('a') < Grapheme::from('b'));
        assert_eq!("好", Grapheme::from('好'));
    }
}
