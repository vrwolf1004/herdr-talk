//! Text rules for names, messages and terminal rows.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Same idea as Python's str.isprintable(): controls, format and private characters a peer
/// must not put on screen are out, and the plain space is the only space kept.
fn is_printable(ch: char) -> bool {
    if ch == ' ' {
        return true;
    }
    if ch.is_control() || ch.is_whitespace() {
        return false;
    }
    !matches!(ch as u32,
        0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x890..=0x891 | 0x8E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F
        | 0xE000..=0xF8FF | 0xFEFF | 0xFFF0..=0xFFFB | 0xFFFE..=0xFFFF
        | 0x110BD | 0x110CD | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A
        | 0xE0000..=0xE0FFF | 0xF0000..=0x10FFFF)
}

/// Replace characters that are not printable with spaces and cut to limit characters.
pub fn clean_text(value: &str, limit: usize) -> String {
    value.chars().take(limit).map(|ch| if is_printable(ch) { ch } else { ' ' }).collect()
}

/// A peer's name from a discovery packet. Same rule as a message sender, then unknown if blank.
pub fn peer_name(value: Option<&str>) -> String {
    let name = clean_text(value.unwrap_or(""), 32);
    if name.trim().is_empty() { "unknown".to_string() } else { name }
}

/// Terminal columns used by text. East Asian wide characters count as two.
pub fn char_width(text: &str) -> usize {
    text.width()
}

/// Split one message into rows of at most cols columns. Breaks at spaces;
/// a word longer than a row is cut by character.
pub fn wrap_line(line: &str, cols: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    for word in if line.is_empty() { " " } else { line }.split(' ') {
        let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if char_width(&candidate) <= cols {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            rows.push(std::mem::take(&mut current));
        }
        let mut width = 0;
        for ch in word.chars() {
            let char_cols = ch.width().unwrap_or(0);
            if width + char_cols > cols && !current.is_empty() {
                rows.push(std::mem::take(&mut current));
                width = 0;
            }
            current.push(ch);
            width += char_cols;
        }
    }
    rows.push(current);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_at_spaces() {
        assert_eq!(wrap_line("one two three", 7), ["one two", "three"]);
    }

    #[test]
    fn cuts_long_word_by_character() {
        let rows = wrap_line("abcdefghij", 4);
        assert!(rows.iter().all(|row| char_width(row) <= 4));
        assert_eq!(rows.concat(), "abcdefghij");
    }

    #[test]
    fn wide_characters_count_as_two() {
        assert_eq!(char_width("가나"), 4);
        assert_eq!(char_width("abc"), 3);
        assert!(wrap_line("가나다라마바사", 4).iter().all(|row| char_width(row) <= 4));
    }

    #[test]
    fn terminal_and_bidi_controls_become_spaces() {
        let cleaned = clean_text("a\x1b[31mred\x07\u{202E}\u{200B}\u{2028}", 100);
        assert_eq!(cleaned, "a [31mred    ");
    }

    #[test]
    fn name_is_cut_to_32_and_blank_becomes_unknown() {
        assert_eq!(peer_name(Some(&"x".repeat(40))).chars().count(), 32);
        for value in [None, Some(""), Some("   "), Some("\x07\x00")] {
            assert_eq!(peer_name(value), "unknown");
        }
    }
}
