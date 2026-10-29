//! Separates Rex's own messages (`ESC _ rex1;<nonce>;<json> ESC \`) from the program's output.
//! A message with another nonce is something a program printed, and passes through untouched.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Resize { cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32 },
    Theme(Value),
    ChildExited { exit_code: Option<i64> },
    Other(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Bytes(Vec<u8>),
    Message(Message),
}

pub struct Demux {
    marker: Vec<u8>,
    /// Bytes held back: a possible start of a marker, or an unfinished message.
    held: Vec<u8>,
}

/// Longest message body accepted before the bytes are treated as program output.
const MAX_MESSAGE: usize = 1 << 20;

impl Demux {
    pub fn new(nonce: u64) -> Self {
        Self { marker: format!("\x1b_rex1;{nonce};").into_bytes(), held: Vec::new() }
    }

    pub fn feed(&mut self, input: &[u8]) -> Vec<Piece> {
        let mut data = std::mem::take(&mut self.held);
        data.extend_from_slice(input);
        let mut pieces = Vec::new();
        let mut plain = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let Some(start) = memchr(0x1b, &data[at..]).map(|i| at + i) else {
                plain.extend_from_slice(&data[at..]);
                break;
            };
            plain.extend_from_slice(&data[at..start]);
            let rest = &data[start..];
            if rest.len() < self.marker.len() {
                if self.marker.starts_with(rest) {
                    self.held = rest.to_vec();
                    break;
                }
            } else if rest.starts_with(&self.marker) {
                let body = &rest[self.marker.len()..];
                match find(body, b"\x1b\\") {
                    Some(end) => {
                        if !plain.is_empty() {
                            pieces.push(Piece::Bytes(std::mem::take(&mut plain)));
                        }
                        pieces.push(Piece::Message(parse(&body[..end])));
                        at = start + self.marker.len() + end + 2;
                        continue;
                    }
                    None if body.len() < MAX_MESSAGE => {
                        self.held = rest.to_vec();
                        break;
                    }
                    None => {}
                }
            }
            plain.push(0x1b);
            at = start + 1;
        }
        if !plain.is_empty() {
            pieces.push(Piece::Bytes(plain));
        }
        pieces
    }
}

fn parse(body: &[u8]) -> Message {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Message::Other(Value::Null);
    };
    match value["type"].as_str() {
        Some("resize") => Message::Resize {
            cols: value["cols"].as_u64().unwrap_or(80) as u16,
            rows: value["rows"].as_u64().unwrap_or(24) as u16,
            cell_width_px: value["cell_width_px"].as_u64().unwrap_or(0) as u32,
            cell_height_px: value["cell_height_px"].as_u64().unwrap_or(0) as u32,
        },
        Some("theme") => Message::Theme(value),
        Some("child_exited") => Message::ChildExited { exit_code: value["exit_code"].as_i64() },
        _ => Message::Other(value),
    }
}

fn memchr(byte: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == byte)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(pieces: &[Piece]) -> Vec<u8> {
        pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Bytes(bytes) => Some(bytes.clone()),
                Piece::Message(_) => None,
            })
            .collect::<Vec<_>>()
            .concat()
    }

    #[test]
    fn messages_with_our_nonce_are_taken_out() {
        let mut demux = Demux::new(5);
        let pieces = demux.feed(b"ab\x1b_rex1;5;{\"type\":\"child_exited\",\"exit_code\":3}\x1b\\cd");
        assert_eq!(
            pieces,
            vec![
                Piece::Bytes(b"ab".to_vec()),
                Piece::Message(Message::ChildExited { exit_code: Some(3) }),
                Piece::Bytes(b"cd".to_vec()),
            ]
        );
    }

    #[test]
    fn a_message_split_across_chunks_is_reassembled() {
        let mut demux = Demux::new(77);
        let whole = b"x\x1b[31m\x1b_rex1;77;{\"type\":\"resize\",\"cols\":100,\"rows\":30}\x1b\\y";
        let mut pieces = Vec::new();
        for byte in whole {
            pieces.extend(demux.feed(&[*byte]));
        }
        assert_eq!(bytes(&pieces), b"x\x1b[31my");
        assert!(pieces.contains(&Piece::Message(Message::Resize {
            cols: 100,
            rows: 30,
            cell_width_px: 0,
            cell_height_px: 0
        })));
    }

    #[test]
    fn look_alikes_from_programs_pass_through() {
        let mut demux = Demux::new(1);
        let fake = b"\x1b_rex1;2;{\"type\":\"child_exited\"}\x1b\\";
        let pieces = demux.feed(fake);
        assert_eq!(bytes(&pieces), fake);
        assert!(pieces.iter().all(|piece| matches!(piece, Piece::Bytes(_))));
    }
}
