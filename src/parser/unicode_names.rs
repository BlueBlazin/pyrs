use std::collections::HashMap;
use std::sync::OnceLock;

static UNICODE_NAME_TABLE: OnceLock<HashMap<&'static str, char>> = OnceLock::new();

const UNICODE_NAME_DATA: &str = include_str!("unicode_names_data.txt");

fn build_unicode_name_table() -> HashMap<&'static str, char> {
    let mut map = HashMap::with_capacity(150_000);
    for line in UNICODE_NAME_DATA.lines() {
        let Some((name, codepoint_hex)) = line.rsplit_once(';') else {
            continue;
        };
        let Some(codepoint) = u32::from_str_radix(codepoint_hex, 16).ok() else {
            continue;
        };
        let Some(ch) = char::from_u32(codepoint) else {
            continue;
        };
        map.insert(name, ch);
    }
    map
}

pub(super) fn lookup_unicode_name(name: &str) -> Option<char> {
    let table = UNICODE_NAME_TABLE.get_or_init(build_unicode_name_table);
    // CPython treats character names case-insensitively for \N{...}.
    let normalized = name.to_ascii_uppercase();
    table.get(normalized.as_str()).copied()
}

#[cfg(test)]
mod tests {
    use super::lookup_unicode_name;

    #[test]
    fn resolves_unicode_name_case_insensitively() {
        assert_eq!(lookup_unicode_name("EMPTY SET"), Some('\u{2205}'));
        assert_eq!(lookup_unicode_name("empty set"), Some('\u{2205}'));
        assert_eq!(lookup_unicode_name("DIGIT NINE"), Some('9'));
    }

    #[test]
    fn returns_none_for_unknown_name() {
        assert_eq!(lookup_unicode_name("NOT A REAL CODEPOINT NAME"), None);
    }
}
