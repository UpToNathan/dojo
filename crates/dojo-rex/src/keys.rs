//! W3C `KeyboardEvent.code` names to libghostty's physical keys.
use crate::ffi::*;

include!(concat!(env!("OUT_DIR"), "/keys.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w3c_codes_map_to_ghostty_keys() {
        assert_eq!(key_from_code("KeyA"), Some(GHOSTTY_KEY_A));
        assert_eq!(key_from_code("ArrowUp"), Some(GHOSTTY_KEY_ARROW_UP));
        assert_eq!(key_from_code("Digit1"), Some(GHOSTTY_KEY_DIGIT_1));
        assert_eq!(key_from_code("BracketLeft"), Some(GHOSTTY_KEY_BRACKET_LEFT));
        assert_eq!(key_from_code("F12"), Some(GHOSTTY_KEY_F12));
        assert_eq!(key_from_code("Enter"), Some(GHOSTTY_KEY_ENTER));
        assert_eq!(key_from_code("Nope"), None);
    }
}
