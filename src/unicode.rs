//! Internal Unicode helpers for surrogate-aware CPython compatibility.
//!
//! Rust `char` values cannot represent surrogate code points directly.
//! We map the surrogate range (`U+D800..=U+DFFF`) into a dedicated internal
//! scalar range so VM strings can round-trip surrogate semantics.

const INTERNAL_SURROGATE_BASE: u32 = 0x10F800;
const INTERNAL_SURROGATE_LIMIT: u32 = INTERNAL_SURROGATE_BASE + 0x800;

pub(crate) fn internal_char_from_codepoint(codepoint: u32) -> Option<char> {
    if (0xD800..=0xDFFF).contains(&codepoint) {
        char::from_u32(INTERNAL_SURROGATE_BASE + (codepoint - 0xD800))
    } else {
        char::from_u32(codepoint)
    }
}

pub(crate) fn surrogate_codepoint_from_internal_char(ch: char) -> Option<u32> {
    let code = ch as u32;
    if (INTERNAL_SURROGATE_BASE..INTERNAL_SURROGATE_LIMIT).contains(&code) {
        Some(0xD800 + (code - INTERNAL_SURROGATE_BASE))
    } else {
        None
    }
}

pub(crate) fn surrogate_code_unit_from_internal_char(ch: char) -> Option<u16> {
    surrogate_codepoint_from_internal_char(ch).map(|codepoint| codepoint as u16)
}

pub(crate) fn canonical_codepoint_for_internal_char(ch: char) -> u32 {
    surrogate_codepoint_from_internal_char(ch).unwrap_or(ch as u32)
}

pub(crate) fn contains_internal_surrogate(text: &str) -> bool {
    text.chars()
        .any(|ch| surrogate_codepoint_from_internal_char(ch).is_some())
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_codepoint_for_internal_char, contains_internal_surrogate,
        internal_char_from_codepoint, surrogate_code_unit_from_internal_char,
        surrogate_codepoint_from_internal_char,
    };

    #[test]
    fn surrogate_mapping_roundtrips_codepoints() {
        let high = internal_char_from_codepoint(0xD83D).expect("high surrogate");
        let low = internal_char_from_codepoint(0xDC0D).expect("low surrogate");
        assert_eq!(surrogate_codepoint_from_internal_char(high), Some(0xD83D));
        assert_eq!(surrogate_codepoint_from_internal_char(low), Some(0xDC0D));
        assert_eq!(surrogate_code_unit_from_internal_char(high), Some(0xD83D));
        assert_eq!(surrogate_code_unit_from_internal_char(low), Some(0xDC0D));
        assert_eq!(canonical_codepoint_for_internal_char(high), 0xD83D);
        assert_eq!(canonical_codepoint_for_internal_char(low), 0xDC0D);
    }

    #[test]
    fn non_surrogate_scalars_are_unchanged() {
        let ch = internal_char_from_codepoint('A' as u32).expect("A");
        assert_eq!(ch, 'A');
        assert_eq!(surrogate_codepoint_from_internal_char(ch), None);
        assert_eq!(canonical_codepoint_for_internal_char(ch), 'A' as u32);
        assert!(!contains_internal_surrogate("x"));
    }

    #[test]
    fn contains_internal_surrogate_detects_mapped_chars() {
        let high = internal_char_from_codepoint(0xD800).expect("high surrogate");
        let text = high.to_string();
        assert!(contains_internal_surrogate(&text));
    }
}
