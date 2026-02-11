use std::collections::HashMap;
use std::sync::OnceLock;

static UNICODE_NAME_TABLE: OnceLock<HashMap<&'static str, UnicodeNameEntry>> = OnceLock::new();

const UNICODE_NAME_DATA: &str = include_str!("unicode_names_data.txt");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnicodeNameEntry {
    Char(char),
    NamedSequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnicodeNameLookup {
    Char(char),
    NamedSequence,
    Unknown,
}

fn build_unicode_name_table() -> HashMap<&'static str, UnicodeNameEntry> {
    let mut map = HashMap::with_capacity(150_000);
    for line in UNICODE_NAME_DATA.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(3, ';');
        let Some(kind) = fields.next() else {
            continue;
        };
        let Some(name) = fields.next() else {
            continue;
        };
        let Some(payload) = fields.next() else {
            continue;
        };
        match kind {
            "C" | "A" => {
                let Some(codepoint) = u32::from_str_radix(payload, 16).ok() else {
                    continue;
                };
                let Some(ch) = char::from_u32(codepoint) else {
                    continue;
                };
                map.insert(name, UnicodeNameEntry::Char(ch));
            }
            "S" => {
                if payload.split_whitespace().next().is_none() {
                    continue;
                }
                map.insert(name, UnicodeNameEntry::NamedSequence);
            }
            _ => continue,
        }
    }
    map
}

pub(super) fn lookup_unicode_name(name: &str) -> UnicodeNameLookup {
    let table = UNICODE_NAME_TABLE.get_or_init(build_unicode_name_table);
    // CPython treats character names case-insensitively for \N{...}.
    let normalized = name.to_ascii_uppercase();
    match table.get(normalized.as_str()).copied() {
        Some(UnicodeNameEntry::Char(ch)) => UnicodeNameLookup::Char(ch),
        Some(UnicodeNameEntry::NamedSequence) => UnicodeNameLookup::NamedSequence,
        None => UnicodeNameLookup::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{UnicodeNameLookup, lookup_unicode_name};

    #[test]
    fn resolves_unicode_name_case_insensitively() {
        assert_eq!(
            lookup_unicode_name("EMPTY SET"),
            UnicodeNameLookup::Char('\u{2205}')
        );
        assert_eq!(
            lookup_unicode_name("empty set"),
            UnicodeNameLookup::Char('\u{2205}')
        );
        assert_eq!(
            lookup_unicode_name("DIGIT NINE"),
            UnicodeNameLookup::Char('9')
        );
    }

    #[test]
    fn resolves_alias_names() {
        assert_eq!(
            lookup_unicode_name("LINE FEED"),
            UnicodeNameLookup::Char('\n')
        );
    }

    #[test]
    fn marks_named_sequences_as_non_scalar() {
        assert_eq!(
            lookup_unicode_name("LATIN SMALL LETTER R WITH TILDE"),
            UnicodeNameLookup::NamedSequence
        );
    }

    #[test]
    fn returns_unknown_for_unknown_name() {
        assert_eq!(
            lookup_unicode_name("NOT A REAL CODEPOINT NAME"),
            UnicodeNameLookup::Unknown
        );
    }
}
