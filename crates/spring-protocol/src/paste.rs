//! Pasted text as the server will take it.
//!
//! The protocol has no message longer than one line, and teiserver cuts each
//! line at a cap without telling anyone (`spring_in.ex`, `String.slice`). So a
//! block of text is sent one line at a time, and a chat line longer than the
//! cap is wrapped into several rather than losing its tail. Chobby does the
//! first half of this (`ParseMultiCommandMessage`) and not the second.

/// One entry per non-empty line, trimmed. A Windows clipboard's `\r\n` is a
/// newline like any other.
pub fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// Splits `line` at whitespace into pieces of at most `max` characters. A word
/// longer than that on its own is cut mid-word; there is nowhere else to cut.
/// Runs of whitespace collapse to one space, which is how chat renders them
/// anyway.
pub fn wrap(line: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    for word in line.split_whitespace().flat_map(|word| cut(word, max)) {
        let word_len = word.chars().count();
        if current_len > 0 && current_len + 1 + word_len > max {
            pieces.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if current_len > 0 {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(&word);
        current_len += word_len;
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// `word` in slices of at most `max` characters, on character boundaries.
fn cut(word: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    chars
        .chunks(max.max(1))
        .map(|chunk| chunk.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_trimmed_and_blank_ones_dropped() {
        assert_eq!(
            lines("!bset a 1\r\n\n  !bset b 2  \n"),
            vec!["!bset a 1", "!bset b 2"]
        );
        assert!(lines("\n \r\n").is_empty());
    }

    #[test]
    fn a_line_that_fits_is_left_alone() {
        assert_eq!(wrap("hello there", 20), vec!["hello there"]);
        assert!(wrap("", 20).is_empty());
    }

    #[test]
    fn wrapping_breaks_between_words_and_fills_each_piece() {
        assert_eq!(
            wrap("one two three four five", 9),
            vec!["one two", "three", "four five"]
        );
        // The cap is inclusive: a piece may be exactly `max` long.
        assert_eq!(wrap("abcd efgh", 9), vec!["abcd efgh"]);
        assert_eq!(wrap("abcd efghi", 9), vec!["abcd", "efghi"]);
    }

    #[test]
    fn a_word_longer_than_the_cap_is_cut_on_a_character_boundary() {
        assert_eq!(wrap("ab ééééé cd", 3), vec!["ab", "ééé", "éé", "cd"]);
    }
}
