//! Bounded binary reading and writing for the display manager's datagrams.
//! Reading is total: every input is an `Ok` or a `DecodeError`, never a panic.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    Truncated,
    Trailing,
    UnknownTag(u8),
    /// The named field is out of range, too long, not UTF-8, or not valid for its type.
    Invalid(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("datagram ends early"),
            DecodeError::Trailing => f.write_str("datagram has trailing bytes"),
            DecodeError::UnknownTag(tag) => write!(f, "unknown tag {tag:#04x}"),
            DecodeError::Invalid(field) => write!(f, "invalid {field}"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub(super) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Reader(bytes)
    }

    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < count {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.0.split_at(count);
        self.0 = tail;
        Ok(head)
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn flag(&mut self, field: &'static str) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Invalid(field)),
        }
    }

    /// A length-prefixed UTF-8 string of at most `max` bytes.
    pub fn text(&mut self, field: &'static str, max: usize) -> Result<&'a str, DecodeError> {
        let length = usize::from(self.u16()?);
        if length > max {
            return Err(DecodeError::Invalid(field));
        }
        std::str::from_utf8(self.bytes(length)?).map_err(|_| DecodeError::Invalid(field))
    }

    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

    pub fn finish(self) -> Result<(), DecodeError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::Trailing)
        }
    }
}

pub(super) struct Writer(Vec<u8>);

impl Writer {
    pub fn new(tag: u8, capacity: usize) -> Self {
        let mut bytes = Vec::with_capacity(1 + capacity);
        bytes.push(tag);
        Writer(bytes)
    }

    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }

    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn flag(&mut self, value: bool) -> &mut Self {
        self.u8(value as u8)
    }

    /// Text longer than `max` bytes is cut at a character boundary, so what is written
    /// is always what `Reader::text` accepts.
    pub fn text(&mut self, text: &str, max: usize) -> &mut Self {
        let text = clip(text, max);
        self.u16(text.len() as u16);
        self.0.extend_from_slice(text.as_bytes());
        self
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.extend_from_slice(bytes);
        self
    }

    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

pub(super) fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cut_on_a_character_boundary() {
        assert_eq!(clip("héllo", 2), "h");
        assert_eq!(clip("héllo", 3), "hé");
        assert_eq!(clip("abc", 9), "abc");
        assert_eq!(clip("😀", 3), "");
    }

    #[test]
    fn reader_refuses_short_long_and_non_utf8_input() {
        assert_eq!(Reader::new(&[1]).u16(), Err(DecodeError::Truncated));
        assert_eq!(
            Reader::new(&[3, 0, b'a']).text("t", 8),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            Reader::new(&[3, 0, b'a', b'b', b'c']).text("t", 2),
            Err(DecodeError::Invalid("t"))
        );
        assert_eq!(
            Reader::new(&[1, 0, 0xff]).text("t", 8),
            Err(DecodeError::Invalid("t"))
        );
        assert_eq!(Reader::new(&[2]).flag("f"), Err(DecodeError::Invalid("f")));
        assert_eq!(Reader::new(&[0]).finish(), Err(DecodeError::Trailing));
    }
}
