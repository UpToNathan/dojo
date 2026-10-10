use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct SearchInput {
    pub text: String,
    pub selection: Range<usize>,
    pub marked: Option<Range<usize>>,
}

impl SearchInput {
    pub fn utf16_offset(&self, byte: usize) -> usize {
        self.text[..byte].encode_utf16().count()
    }

    pub fn byte_offset(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (byte, character) in self.text.char_indices() {
            if units >= utf16 {
                return byte;
            }
            units += character.len_utf16();
        }
        self.text.len()
    }

    pub fn from_utf16(&self, range: Range<usize>) -> Range<usize> {
        self.byte_offset(range.start)..self.byte_offset(range.end)
    }

    pub fn to_utf16(&self, range: Range<usize>) -> Range<usize> {
        self.utf16_offset(range.start)..self.utf16_offset(range.end)
    }

    pub fn replace(&mut self, utf16: Option<Range<usize>>, text: &str) -> Range<usize> {
        let range = utf16
            .map(|range| self.from_utf16(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        self.text.replace_range(range.clone(), text);
        let inserted = range.start..range.start + text.len();
        self.selection = inserted.end..inserted.end;
        self.marked = None;
        inserted
    }

    pub fn backspace(&mut self) {
        if self.selection.is_empty() {
            let end = self.selection.end;
            let start = self.text[..end]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(byte, _)| byte);
            self.selection = start..end;
        }
        self.replace(None, "");
    }

    pub fn delete(&mut self) {
        if self.selection.is_empty() {
            let start = self.selection.start;
            let length = self.text[start..]
                .graphemes(true)
                .next()
                .map_or(0, str::len);
            self.selection = start..start + length;
        }
        self.replace(None, "");
    }

    pub fn move_cursor(&mut self, right: bool) {
        let byte = if !self.selection.is_empty() {
            if right {
                self.selection.end
            } else {
                self.selection.start
            }
        } else if right {
            self.selection.end
                + self.text[self.selection.end..]
                    .graphemes(true)
                    .next()
                    .map_or(0, str::len)
        } else {
            self.text[..self.selection.start]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(byte, _)| byte)
        };
        self.selection = byte..byte;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_graphemes_and_round_trips_utf16_ranges() {
        let mut input = SearchInput::default();
        input.replace(None, "a😀e\u{301}");
        assert_eq!(input.from_utf16(1..3), 1..5);
        assert_eq!(input.to_utf16(1..5), 1..3);
        input.backspace();
        assert_eq!(input.text, "a😀");
        input.backspace();
        assert_eq!(input.text, "a");
        input.move_cursor(false);
        input.delete();
        assert!(input.text.is_empty());
    }

    #[test]
    fn composition_replaces_the_marked_text_instead_of_appending() {
        let mut input = SearchInput::default();
        input.marked = Some(input.replace(None, "に"));
        input.marked = Some(input.replace(None, "日本"));
        input.replace(None, "日本語");
        assert_eq!(input.text, "日本語");
        assert_eq!(input.marked, None);
    }
}
