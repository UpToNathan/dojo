use std::fmt;

use zeroize::Zeroize;

/// The typed password. No Clone, Display or Serialize.
/// Fixed capacity, so pushing never reallocates and leaves no stale copy behind.
pub struct Password {
    bytes: Box<[u8; Password::CAPACITY]>,
    len: usize,
    chars: usize,
}

impl Password {
    pub const CAPACITY: usize = 512;

    pub fn new() -> Self {
        let bytes = Box::new([0u8; Self::CAPACITY]);
        // Best effort: keeps the page out of swap. Never munlocked, because
        // another live Password may share the page and mlock does not nest.
        unsafe { libc::mlock(bytes.as_ptr().cast(), Self::CAPACITY) };
        Password {
            bytes,
            len: 0,
            chars: 0,
        }
    }

    /// False when full, or for NUL, which PAM's C strings cannot carry.
    pub fn push(&mut self, c: char) -> bool {
        let width = c.len_utf8();
        if c == '\0' || self.len + width > Self::CAPACITY {
            return false;
        }
        c.encode_utf8(&mut self.bytes[self.len..self.len + width]);
        self.len += width;
        self.chars += 1;
        true
    }

    pub fn pop(&mut self) {
        let Some(last) = (0..self.len).rev().find(|&i| self.bytes[i] & 0xC0 != 0x80) else {
            return;
        };
        self.bytes[last..self.len].zeroize();
        self.len = last;
        self.chars -= 1;
    }

    pub fn char_count(&self) -> usize {
        self.chars
    }

    /// The only read of the secret. Its one caller is `worker::wire::send_answer`.
    pub(crate) fn expose(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Default for Password {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// Keeps a secret out of core dumps and same-user ptrace. Both the locker and
/// the PAM helper call it before they hold a password.
pub fn protect_process() {
    let no_core = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        libc::setrlimit(libc::RLIMIT_CORE, &no_core);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Password {
        let mut password = Password::new();
        for c in text.chars() {
            assert!(password.push(c));
        }
        password
    }

    #[test]
    fn push_and_pop_track_characters_not_bytes() {
        let mut password = typed("aé€😀");
        assert_eq!(password.char_count(), 4);
        assert_eq!(password.expose(), "aé€😀".as_bytes());
        password.pop();
        assert_eq!(password.expose(), "aé€".as_bytes());
        password.pop();
        password.pop();
        assert_eq!(password.expose(), b"a");
        assert_eq!(password.char_count(), 1);
        password.pop();
        password.pop();
        assert_eq!(password.expose(), b"");
        assert_eq!(password.char_count(), 0);
    }

    #[test]
    fn pop_zeroes_the_removed_bytes() {
        let mut password = typed("ab€");
        password.pop();
        assert_eq!(&password.bytes[..6], &[b'a', b'b', 0, 0, 0, 0]);
    }

    #[test]
    fn push_refuses_past_capacity_without_partial_writes() {
        let mut password = Password::new();
        for _ in 0..Password::CAPACITY - 1 {
            assert!(password.push('x'));
        }
        assert!(!password.push('é'));
        assert!(password.push('y'));
        assert!(!password.push('z'));
        assert_eq!(password.char_count(), Password::CAPACITY);
        assert_eq!(password.expose().len(), Password::CAPACITY);
        assert_eq!(password.expose()[Password::CAPACITY - 1], b'y');
    }

    #[test]
    fn push_refuses_nul() {
        let mut password = typed("a");
        assert!(!password.push('\0'));
        assert_eq!(password.expose(), b"a");
    }

    #[test]
    fn debug_is_redacted() {
        let password = typed("hunter2");
        let shown = format!("{password:?} {password:#?}");
        assert_eq!(shown, "Password(<redacted>) Password(<redacted>)");
    }
}
