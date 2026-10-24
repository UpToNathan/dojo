//! 12 Dev utilities as inline rows. See .scratch/launcher-features/issues/12-dev-utilities-rows.md.
//!
//! [`evaluate`] answers only for an explicit trigger word at the start of the query, followed by
//! whitespace or the end, so an application called `uuidgen-gui` or `base64-tool` is not
//! mistaken for one. Randomness and the clock come from a [`DevEnv`], which keeps every utility
//! deterministic under test; [`SystemEnv`] is the real one.
//!
//! | Trigger | Rows |
//! |---|---|
//! | `uuid` | a v4 and a v7 UUID; `uuid4` and `uuid7` give one |
//! | `base64 <text>`, `unbase64 <text>` | encode (plus URL-safe when it differs); decode, accepting either alphabet and optional padding |
//! | `sha1`, `sha256`, `sha512`, `md5 <text>` | lowercase hex digest of the text's UTF-8 bytes |
//! | `urlencode <text>`, `urldecode <text>` | percent-encoding; decoding also turns `+` into a space |
//! | `ts <seconds or ms>` | ISO 8601 in UTC and a relative time; at 100 000 000 000 and above the number is milliseconds |
//! | `ts <ISO 8601 date>` | Unix seconds and milliseconds; no zone means UTC |
//! | `ts`, `ts now` | the current time in all three |
//! | `json <text>` | pretty-printed and minified, keeping key order, escapes and number spelling |
//! | `snake`, `camel`, `kebab`, `pascal`, `upper`, `lower`, `title <text>` | the converted text |
//! | `pw [length]` | a password with symbols and an alphanumeric one; length 4 to 128, default 20 |
//! | `lorem [words]` | the classic passage, 1 to 1000 words, default 50 |
//!
//! A recognised trigger with unusable input (bad base64, bad JSON, a length out of range) returns
//! `None`, the same silence as a half-typed calculator expression. Rows never contain newlines in
//! `display`; multi-line results (pretty JSON) collapse to one line there while `copy` stays exact.
//! Case words such as `title` and `lower` also start ordinary searches (`title bar`); the launcher
//! shows these rows as extras below the application results, never in place of them.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use sha2::{Digest as _, Sha256, Sha512};

/// One inline row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevAnswer {
    /// What the row is, such as `SHA-256` or `Unix seconds`.
    pub label: String,
    /// The one-line text the row shows.
    pub display: String,
    /// What Enter copies, exactly.
    pub copy: String,
}

impl DevAnswer {
    fn new(label: impl Into<String>, text: String) -> Self {
        Self {
            label: label.into(),
            display: single_line(&text),
            copy: text,
        }
    }
}

/// What the utilities need from the outside world.
pub trait DevEnv {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
    /// Fills `buffer` with cryptographically secure random bytes. Returns false when none could be
    /// produced; the utilities that need them then answer `None`.
    fn fill_random(&self, buffer: &mut [u8]) -> bool;
}

/// The real clock and the kernel's random source (`/dev/urandom`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemEnv;

impl DevEnv for SystemEnv {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            })
    }

    fn fill_random(&self, buffer: &mut [u8]) -> bool {
        use std::io::Read as _;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(buffer))
            .is_ok()
    }
}

const MAX_QUERY_BYTES: usize = 100_000;

/// The rows for a query that starts with a dev-utility trigger, or `None` for anything else.
pub fn evaluate(query: &str, env: &impl DevEnv) -> Option<Vec<DevAnswer>> {
    let query = query.trim();
    if query.is_empty() || query.len() > MAX_QUERY_BYTES {
        return None;
    }
    let (keyword, text) = match query.split_once(char::is_whitespace) {
        Some((keyword, rest)) => (keyword, rest.trim()),
        None => (query, ""),
    };
    let keyword = keyword.to_lowercase();
    let rows = match keyword.as_str() {
        "uuid" if text.is_empty() => {
            let mut rows = uuid_v4(env)?;
            rows.extend(uuid_v7(env)?);
            rows
        }
        "uuid4" if text.is_empty() => uuid_v4(env)?,
        "uuid7" if text.is_empty() => uuid_v7(env)?,
        "base64" if !text.is_empty() => base64_encode(text),
        "unbase64" if !text.is_empty() => base64_decode(text)?,
        "sha1" | "sha256" | "sha512" | "md5" if !text.is_empty() => hash(&keyword, text),
        "urlencode" if !text.is_empty() => vec![DevAnswer::new("URL encoded", url_encode(text))],
        "urldecode" if !text.is_empty() => vec![DevAnswer::new("URL decoded", url_decode(text)?)],
        "ts" | "timestamp" => timestamp(text, env)?,
        "json" if !text.is_empty() => json(text)?,
        "snake" | "camel" | "kebab" | "pascal" | "upper" | "lower" | "title"
            if !text.is_empty() =>
        {
            vec![convert_case(&keyword, text)?]
        }
        "pw" => password(text, env)?,
        "lorem" => lorem(text)?,
        _ => return None,
    };
    (!rows.is_empty()).then_some(rows)
}

/// Collapses a multi-line result for the one-line row.
fn single_line(text: &str) -> String {
    if !text.contains('\n') {
        return text.to_owned();
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ------------------------------------------------------------ randomness

fn random_array<const N: usize>(env: &impl DevEnv) -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    env.fill_random(&mut bytes).then_some(bytes)
}

/// A uniform number in `0..bound` without modulo bias.
fn random_below(env: &impl DevEnv, bound: usize) -> Option<usize> {
    debug_assert!(bound > 0);
    let bound = bound as u32;
    let limit = u32::MAX - u32::MAX % bound;
    for _ in 0..64 {
        let value = u32::from_le_bytes(random_array::<4>(env)?);
        if value < limit {
            return Some((value % bound) as usize);
        }
    }
    // A broken generator that keeps returning the top of the range still terminates.
    Some((u32::from_le_bytes(random_array::<4>(env)?) % bound) as usize)
}

// ------------------------------------------------------------------ uuid

fn uuid_v4(env: &impl DevEnv) -> Option<Vec<DevAnswer>> {
    let uuid = uuid::Builder::from_random_bytes(random_array::<16>(env)?).into_uuid();
    Some(vec![DevAnswer::new("UUID v4", uuid.to_string())])
}

fn uuid_v7(env: &impl DevEnv) -> Option<Vec<DevAnswer>> {
    let uuid = uuid::Builder::from_unix_timestamp_millis(env.now_ms(), &random_array::<10>(env)?)
        .into_uuid();
    Some(vec![DevAnswer::new("UUID v7", uuid.to_string())])
}

// ---------------------------------------------------------------- base64

fn base64_encode(text: &str) -> Vec<DevAnswer> {
    let standard = STANDARD.encode(text);
    let url_safe = URL_SAFE_NO_PAD.encode(text);
    let mut rows = vec![DevAnswer::new("Base64", standard.clone())];
    if url_safe != standard.trim_end_matches('=') {
        rows.push(DevAnswer::new("Base64 URL-safe", url_safe));
    }
    rows
}

/// Accepts the standard and URL-safe alphabets, with or without padding, and ignores whitespace.
/// Bytes that are not UTF-8 text are refused: they would not paste as text.
fn base64_decode(text: &str) -> Option<Vec<DevAnswer>> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let config =
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let engine = if compact.contains(['-', '_']) {
        GeneralPurpose::new(&base64::alphabet::URL_SAFE, config)
    } else {
        GeneralPurpose::new(&base64::alphabet::STANDARD, config)
    };
    let bytes = engine.decode(compact).ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    Some(vec![DevAnswer::new("Base64 decoded", decoded)])
}

// ---------------------------------------------------------------- hashes

fn hash(algorithm: &str, text: &str) -> Vec<DevAnswer> {
    let bytes = text.as_bytes();
    let (label, digest) = match algorithm {
        "sha256" => ("SHA-256", hex(&Sha256::digest(bytes))),
        "sha512" => ("SHA-512", hex(&Sha512::digest(bytes))),
        "sha1" => ("SHA-1", hex(&sha1(bytes))),
        _ => ("MD5", hex(&md5(bytes))),
    };
    vec![DevAnswer::new(label, digest)]
}

/// Pads a message the way SHA-1 and MD5 do (they differ only in the length's byte order).
fn md_pad(message: &[u8], big_endian_length: bool) -> Vec<u8> {
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    let bits = (message.len() as u64).wrapping_mul(8);
    padded.extend_from_slice(&if big_endian_length {
        bits.to_be_bytes()
    } else {
        bits.to_le_bytes()
    });
    padded
}

/// SHA-1 (FIPS 180-4). Kept for checksums people still publish; not for security.
fn sha1(message: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    for block in md_pad(message, true).chunks_exact(64) {
        let mut w = [0u32; 80];
        for (word, bytes) in w.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 20];
    for (bytes, word) in digest.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// MD5 (RFC 1321). Kept for checksums people still publish; not for security.
fn md5(message: &[u8]) -> [u8; 16] {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(2^32 * abs(sin(i + 1))).
    let constants: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in md_pad(message, false).chunks_exact(64) {
        let mut m = [0u32; 16];
        for (word, bytes) in m.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(constants[i])
                .wrapping_add(m[g])
                .rotate_left(SHIFTS[i]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 16];
    for (bytes, word) in digest.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    digest
}

// ------------------------------------------------------------------- url

fn url_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// `%XX` escapes and `+` as a space. A stray `%` or non-UTF-8 result is refused.
fn url_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let pair = text.get(i + 1..i + 3)?;
                if !pair.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                decoded.push(u8::from_str_radix(pair, 16).ok()?);
                i += 3;
            }
            b'+' => {
                decoded.push(b' ');
                i += 1;
            }
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

// ------------------------------------------------------------ timestamps

const MS_PER_DAY: i64 = 86_400_000;
/// 0001-01-01T00:00:00Z and 9999-12-31T23:59:59.999Z.
const MIN_MS: i64 = -62_135_596_800_000;
const MAX_MS: i64 = 253_402_300_799_999;
/// Numbers at or above this many seconds read as milliseconds (1973 as ms, year 5138 as s).
const MS_THRESHOLD: i64 = 100_000_000_000;

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `2023-11-14T22:13:20Z`, with `.123` before the `Z` when there are milliseconds.
fn iso_utc(ms: i64) -> String {
    let (year, month, day) = civil_from_days(ms.div_euclid(MS_PER_DAY));
    let in_day = ms.rem_euclid(MS_PER_DAY);
    let (hours, minutes, seconds, millis) = (
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000,
    );
    let fraction = if millis == 0 {
        String::new()
    } else {
        format!(".{millis:03}")
    };
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}{fraction}Z")
}

fn relative(ms: i64, now_ms: i64) -> String {
    let difference = ms - now_ms;
    let seconds = difference.abs() / 1000;
    if seconds < 5 {
        return "just now".to_owned();
    }
    let (amount, unit) = match seconds {
        0..=59 => (seconds, "second"),
        60..=3599 => (seconds / 60, "minute"),
        3600..=86_399 => (seconds / 3600, "hour"),
        86_400..=2_591_999 => (seconds / 86_400, "day"),
        2_592_000..=31_535_999 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    let plural = if amount == 1 { "" } else { "s" };
    if difference < 0 {
        format!("{amount} {unit}{plural} ago")
    } else {
        format!("in {amount} {unit}{plural}")
    }
}

fn timestamp(text: &str, env: &impl DevEnv) -> Option<Vec<DevAnswer>> {
    let now = i64::try_from(env.now_ms()).ok()?;
    if text.is_empty() || text.eq_ignore_ascii_case("now") {
        return Some(vec![
            DevAnswer::new("Unix seconds", now.div_euclid(1000).to_string()),
            DevAnswer::new("Unix milliseconds", now.to_string()),
            DevAnswer::new("ISO 8601 (UTC)", iso_utc(now)),
        ]);
    }
    if let Some(ms) = parse_epoch(text) {
        let seconds_label = ms.1;
        let ms = ms.0;
        if !(MIN_MS..=MAX_MS).contains(&ms) {
            return None;
        }
        return Some(vec![
            DevAnswer::new(format!("{seconds_label} to ISO 8601 (UTC)"), iso_utc(ms)),
            DevAnswer::new("Relative", relative(ms, now)),
        ]);
    }
    let ms = parse_iso(text)?;
    if !(MIN_MS..=MAX_MS).contains(&ms) {
        return None;
    }
    Some(vec![
        DevAnswer::new("Unix seconds", ms.div_euclid(1000).to_string()),
        DevAnswer::new("Unix milliseconds", ms.to_string()),
    ])
}

/// A plain integer of seconds or milliseconds (optionally negative, optionally with fractional
/// seconds). Returns milliseconds and which unit it was read as.
fn parse_epoch(text: &str) -> Option<(i64, &'static str)> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || whole.len() > 18 || fraction.is_some_and(|f| !digits(f)) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let (ms, unit) = if whole >= MS_THRESHOLD {
        (whole, "Unix milliseconds")
    } else {
        let fraction = fraction.unwrap_or("0");
        let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
        (whole * 1000 + millis, "Unix seconds")
    };
    Some((if negative { -ms } else { ms }, unit))
}

/// `YYYY-MM-DD`, then optionally `T` or a space and `HH:MM[:SS[.fraction]]`, then optionally
/// `Z` or `+HH:MM`, `+HHMM`, `+HH`. Returns milliseconds since the epoch.
fn parse_iso(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let number = |from: usize, len: usize| -> Option<i64> {
        let slice = bytes.get(from..from + len)?;
        slice
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| slice.iter().fold(0i64, |n, d| n * 10 + i64::from(d - b'0')))
    };
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let (year, month, day) = (number(0, 4)?, number(5, 2)?, number(8, 2)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || year < 1 {
        return None;
    }
    let mut ms = days_from_civil(year, month, day) * MS_PER_DAY;
    let mut i = 10;
    if i < bytes.len() && matches!(bytes[i], b'T' | b't' | b' ') {
        i += 1;
        let (hours, minutes) = (number(i, 2)?, number(i + 3, 2)?);
        if bytes.get(i + 2) != Some(&b':') || hours > 23 || minutes > 59 {
            return None;
        }
        i += 5;
        let mut seconds = 0;
        let mut millis = 0;
        if bytes.get(i) == Some(&b':') {
            seconds = number(i + 1, 2)?;
            if seconds > 59 {
                return None;
            }
            i += 3;
            if bytes.get(i) == Some(&b'.') {
                let start = i + 1;
                let mut end = start;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
                if end == start {
                    return None;
                }
                millis = format!("{:0<3}", &text[start..end])[..3].parse().ok()?;
                i = end;
            }
        }
        ms += hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + millis;
        if bytes.get(i) == Some(&b' ') && matches!(bytes.get(i + 1), Some(b'+' | b'-')) {
            i += 1;
        }
        match bytes.get(i) {
            None => {}
            Some(b'Z' | b'z') if i + 1 == bytes.len() => {}
            Some(sign @ (b'+' | b'-')) => {
                let offset_hours = number(i + 1, 2)?;
                let rest = &text[i + 3..];
                let offset_minutes = match rest {
                    "" => 0,
                    _ if rest.len() == 3 && rest.starts_with(':') => number(i + 4, 2)?,
                    _ if rest.len() == 2 => number(i + 3, 2)?,
                    _ => return None,
                };
                if offset_hours > 23 || offset_minutes > 59 {
                    return None;
                }
                let offset = offset_hours * 3_600_000 + offset_minutes * 60_000;
                ms -= if *sign == b'+' { offset } else { -offset };
            }
            Some(_) => return None,
        }
    } else if i != bytes.len() {
        return None;
    }
    Some(ms)
}

// ------------------------------------------------------------------ json

/// Pretty-prints and minifies by re-spacing the text itself rather than rebuilding a value, so
/// key order, string escapes and number spelling (including integers beyond f64) survive.
fn json(text: &str) -> Option<Vec<DevAnswer>> {
    serde_json::from_str::<serde::de::IgnoredAny>(text).ok()?;
    Some(vec![
        DevAnswer::new("JSON pretty", reformat_json(text, true)),
        DevAnswer::new("JSON minified", reformat_json(text, false)),
    ])
}

/// `text` must already be valid JSON.
fn reformat_json(text: &str, pretty: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = text.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                let closes = if c == '{' { '}' } else { ']' };
                // Empty containers stay `{}` and `[]`.
                let mut lookahead = chars.clone();
                let next = lookahead.find(|c| !c.is_whitespace());
                if next == Some(closes) {
                    for skipped in chars.by_ref() {
                        if skipped == closes {
                            break;
                        }
                    }
                    out.push(closes);
                } else {
                    depth += 1;
                    if pretty {
                        newline(&mut out, depth);
                    }
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                if pretty {
                    newline(&mut out, depth);
                }
                out.push(c);
            }
            ',' => {
                out.push(c);
                if pretty {
                    newline(&mut out, depth);
                }
            }
            ':' => {
                out.push(c);
                if pretty {
                    out.push(' ');
                }
            }
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

// ----------------------------------------------------------------- case

fn split_words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(&previous) = i.checked_sub(1).and_then(|p| chars.get(p)) {
            let starts_word = c.is_uppercase()
                && !current.is_empty()
                && (previous.is_lowercase()
                    || previous.is_ascii_digit()
                    // `HTTPServer` splits before the `S`, where an acronym ends.
                    || (previous.is_uppercase()
                        && chars.get(i + 1).is_some_and(|next| next.is_lowercase())));
            if starts_word {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

fn convert_case(style: &str, text: &str) -> Option<DevAnswer> {
    let (label, converted) = match style {
        "upper" => ("UPPER CASE", text.to_uppercase()),
        "lower" => ("lower case", text.to_lowercase()),
        "title" => (
            "Title Case",
            text.split_whitespace()
                .map(capitalize)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => {
            let words = split_words(text);
            if words.is_empty() {
                return None;
            }
            let lower = |word: &String| word.to_lowercase();
            match style {
                "snake" => (
                    "snake_case",
                    words.iter().map(lower).collect::<Vec<_>>().join("_"),
                ),
                "kebab" => (
                    "kebab-case",
                    words.iter().map(lower).collect::<Vec<_>>().join("-"),
                ),
                "pascal" => ("PascalCase", words.iter().map(|w| capitalize(w)).collect()),
                _ => (
                    "camelCase",
                    words
                        .iter()
                        .enumerate()
                        .map(|(i, word)| {
                            if i == 0 {
                                lower(word)
                            } else {
                                capitalize(word)
                            }
                        })
                        .collect(),
                ),
            }
        }
    };
    Some(DevAnswer::new(label, converted))
}

// ------------------------------------------------------------- password

const DEFAULT_PASSWORD_LENGTH: usize = 20;
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*-_+=?";

fn password(arguments: &str, env: &impl DevEnv) -> Option<Vec<DevAnswer>> {
    let length = if arguments.is_empty() {
        DEFAULT_PASSWORD_LENGTH
    } else {
        arguments.parse::<usize>().ok()?
    };
    if !(4..=128).contains(&length) {
        return None;
    }
    let with_symbols = generate_password(env, &[LOWER, UPPER, DIGITS, SYMBOLS], length)?;
    let alphanumeric = generate_password(env, &[LOWER, UPPER, DIGITS], length)?;
    Some(vec![
        DevAnswer::new(format!("Password ({length})"), with_symbols),
        DevAnswer::new(format!("Alphanumeric password ({length})"), alphanumeric),
    ])
}

/// Uniformly random characters with at least one from every class, shuffled.
fn generate_password(env: &impl DevEnv, classes: &[&str], length: usize) -> Option<String> {
    let all: Vec<char> = classes.iter().flat_map(|class| class.chars()).collect();
    let mut chars = Vec::with_capacity(length);
    for class in classes {
        let class: Vec<char> = class.chars().collect();
        chars.push(class[random_below(env, class.len())?]);
    }
    while chars.len() < length {
        chars.push(all[random_below(env, all.len())?]);
    }
    for i in (1..chars.len()).rev() {
        chars.swap(i, random_below(env, i + 1)?);
    }
    Some(chars.into_iter().collect())
}

// ----------------------------------------------------------------- lorem

const LOREM: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.";
const DEFAULT_LOREM_WORDS: usize = 50;

/// The first `n` words of the classic passage, repeating it as needed, ending in a full stop.
fn lorem(arguments: &str) -> Option<Vec<DevAnswer>> {
    let count = if arguments.is_empty() {
        DEFAULT_LOREM_WORDS
    } else {
        arguments.parse::<usize>().ok()?
    };
    if !(1..=1000).contains(&count) {
        return None;
    }
    let words: Vec<&str> = LOREM.split_whitespace().collect();
    let mut text = (0..count)
        .map(|i| words[i % words.len()])
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed_len = text.trim_end_matches(['.', ',']).len();
    text.truncate(trimmed_len);
    text.push('.');
    let plural = if count == 1 { "" } else { "s" };
    Some(vec![DevAnswer::new(
        format!("Lorem ipsum ({count} word{plural})"),
        text,
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A fixed clock and a SplitMix64 stream, so every run sees the same "random" bytes.
    struct TestEnv {
        now_ms: u64,
        state: Cell<u64>,
        random: bool,
    }

    impl TestEnv {
        fn new() -> Self {
            // 2023-11-14T22:13:20Z.
            Self {
                now_ms: 1_700_000_000_000,
                state: Cell::new(42),
                random: true,
            }
        }

        fn without_randomness() -> Self {
            Self {
                random: false,
                ..Self::new()
            }
        }

        fn next(&self) -> u64 {
            let state = self.state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
            self.state.set(state);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
    }

    impl DevEnv for TestEnv {
        fn now_ms(&self) -> u64 {
            self.now_ms
        }

        fn fill_random(&self, buffer: &mut [u8]) -> bool {
            if !self.random {
                return false;
            }
            for chunk in buffer.chunks_mut(8) {
                chunk.copy_from_slice(&self.next().to_le_bytes()[..chunk.len()]);
            }
            true
        }
    }

    fn rows(query: &str) -> Vec<DevAnswer> {
        evaluate(query, &TestEnv::new())
            .unwrap_or_else(|| panic!("{query:?} should be a dev utility"))
    }

    fn copy(query: &str) -> String {
        rows(query).remove(0).copy
    }

    fn none(query: &str) {
        assert_eq!(evaluate(query, &TestEnv::new()), None, "{query:?}");
    }

    #[test]
    fn answers_have_label_display_and_copy() {
        let row = rows("sha256 abc").remove(0);
        assert_eq!(row.label, "SHA-256");
        assert_eq!(row.display, row.copy);
        assert_eq!(row.copy.len(), 64);
    }

    #[test]
    fn triggers_are_explicit_and_unambiguous() {
        for query in [
            "uuidgen-gui",
            "uuidgen",
            "uuids",
            "uuid foo",
            "uuid4 x",
            "base64",
            "base64-tool hello",
            "base64hello",
            "unbase64",
            "sha256",
            "sha256sum abc",
            "md5sum abc",
            "json",
            "jsonnet x",
            "snake",
            "snake-game",
            "uppercase hi",
            "tsc",
            "tsconfig x",
            "pwgen",
            "pwd",
            "pwsafe",
            "loremipsum",
            "lorem-ipsum",
            "firefox",
            "Visual Studio Code",
            "",
            "   ",
            "sha",
            "urlencoded x",
            "urlencode",
            "urldecode",
        ] {
            none(query);
        }
    }

    #[test]
    fn triggers_ignore_case_and_surrounding_space() {
        assert_eq!(copy("  SHA256   abc  "), copy("sha256 abc"));
        assert_eq!(copy("Base64 hi"), "aGk=");
        assert!(evaluate("UUID", &TestEnv::new()).is_some());
    }

    #[test]
    fn uuids_are_well_formed_and_deterministic() {
        let env = TestEnv::new();
        let all = evaluate("uuid", &env).unwrap();
        assert_eq!(
            all.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            ["UUID v4", "UUID v7"]
        );
        let v4 = uuid::Uuid::parse_str(&all[0].copy).unwrap();
        assert_eq!(v4.get_version(), Some(uuid::Version::Random));
        assert_eq!(v4.get_variant(), uuid::Variant::RFC4122);
        let v7 = uuid::Uuid::parse_str(&all[1].copy).unwrap();
        assert_eq!(v7.get_version(), Some(uuid::Version::SortRand));
        assert_eq!(v7.get_timestamp().unwrap().to_unix().0, 1_700_000_000);
        assert!(
            all[0]
                .copy
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-')
        );
        assert_eq!(all[0].copy, all[0].copy.to_lowercase());
        assert_eq!(evaluate("uuid", &TestEnv::new()), Some(all));
    }

    #[test]
    fn uuid_variants_and_distinct_values() {
        let env = TestEnv::new();
        let first = evaluate("uuid4", &env).unwrap();
        let second = evaluate("uuid4", &env).unwrap();
        assert_eq!(first.len(), 1);
        assert_ne!(first, second);
        assert_eq!(evaluate("uuid7", &env).unwrap()[0].label, "UUID v7");
    }

    #[test]
    fn random_utilities_give_up_without_randomness() {
        let env = TestEnv::without_randomness();
        for query in ["uuid", "uuid4", "uuid7", "pw", "pw 12"] {
            assert_eq!(evaluate(query, &env), None, "{query:?}");
        }
        assert!(evaluate("sha256 abc", &env).is_some());
        assert!(evaluate("ts now", &env).is_some());
    }

    #[test]
    fn base64_encodes_text_and_unicode() {
        assert_eq!(copy("base64 hello"), "aGVsbG8=");
        assert_eq!(copy("base64 hello world"), "aGVsbG8gd29ybGQ=");
        assert_eq!(copy("base64 héllo ✓"), "aMOpbGxvIOKckw==");
        assert_eq!(rows("base64 hello").len(), 1);
    }

    #[test]
    fn base64_offers_a_url_safe_row_only_when_it_differs() {
        let both = rows("base64 ??>>");
        assert_eq!(both[0].copy, "Pz8+Pg==");
        assert_eq!(both[1].label, "Base64 URL-safe");
        assert_eq!(both[1].copy, "Pz8-Pg");
    }

    #[test]
    fn unbase64_decodes_either_alphabet_with_or_without_padding() {
        assert_eq!(copy("unbase64 aGVsbG8="), "hello");
        assert_eq!(copy("unbase64 aGVsbG8"), "hello");
        assert_eq!(copy("unbase64 Pz8-Pg"), "??>>");
        assert_eq!(copy("unbase64 Pz8+Pg=="), "??>>");
        assert_eq!(copy("unbase64 aMOpbGxvIOKckw=="), "héllo ✓");
        assert_eq!(copy("unbase64 aGVs bG8="), "hello");
        assert_eq!(rows("unbase64 aGk=")[0].label, "Base64 decoded");
    }

    #[test]
    fn unbase64_rejects_invalid_and_binary_input() {
        for query in [
            "unbase64 !!!",
            "unbase64 a",
            "unbase64 aGVsbG8===",
            "unbase64 aGV$bG8=",
            "unbase64 /w==",
            "unbase64 +/+/",
        ] {
            none(query);
        }
    }

    #[test]
    fn hashes_match_published_vectors() {
        assert_eq!(
            copy("sha256 abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            copy("sha512 abc"),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(copy("sha1 abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(copy("md5 abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            copy("md5 The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(rows("md5 abc")[0].label, "MD5");
        assert_eq!(rows("sha1 abc")[0].label, "SHA-1");
        assert_eq!(rows("sha512 abc")[0].label, "SHA-512");
    }

    #[test]
    fn hand_written_digests_handle_empty_and_multi_block_messages() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let digits = "1234567890".repeat(8);
        assert_eq!(
            hex(&md5(digits.as_bytes())),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
        // 55, 56 and 64 bytes straddle the padding boundary.
        assert_eq!(
            hex(&sha1(&[b'a'; 55])),
            "c1c8bbdc22796e28c0e15163d20899b65621d65a"
        );
        assert_eq!(hex(&md5(&[b'a'; 56])), "3b0c8ac703f828b04c6c197006d17218");
        assert_eq!(
            hex(&sha1(&[b'a'; 64])),
            "0098ba824b5c16427bd7a1122a5a442a25ec644d"
        );
    }

    #[test]
    fn hashes_use_utf8_bytes_of_the_text_as_typed() {
        assert_eq!(
            copy("sha256 héllo"),
            hex(&Sha256::digest("héllo".as_bytes()))
        );
        assert_ne!(copy("sha256 a b"), copy("sha256 ab"));
        assert_eq!(copy("sha256 a  b"), hex(&Sha256::digest(b"a  b")));
    }

    #[test]
    fn url_encoding() {
        assert_eq!(copy("urlencode hello world"), "hello%20world");
        assert_eq!(copy("urlencode a&b=c/d?e"), "a%26b%3Dc%2Fd%3Fe");
        assert_eq!(copy("urlencode safe-_.~AZaz09"), "safe-_.~AZaz09");
        assert_eq!(copy("urlencode héllo ✓"), "h%C3%A9llo%20%E2%9C%93");
        assert_eq!(rows("urlencode a b")[0].label, "URL encoded");
    }

    #[test]
    fn url_decoding() {
        assert_eq!(copy("urldecode hello%20world"), "hello world");
        assert_eq!(copy("urldecode a+b"), "a b");
        assert_eq!(copy("urldecode h%C3%A9llo%20%E2%9C%93"), "héllo ✓");
        assert_eq!(copy("urldecode plain"), "plain");
        assert_eq!(copy("urldecode %2f%2F"), "//");
    }

    #[test]
    fn url_decoding_rejects_bad_escapes_and_bad_utf8() {
        for query in [
            "urldecode 100%",
            "urldecode %zz",
            "urldecode %2",
            "urldecode %ff",
            "urldecode %c3",
            "urldecode %é1",
        ] {
            none(query);
        }
    }

    #[test]
    fn url_round_trip() {
        for text in [
            "a b",
            "a+b",
            "ünïcode ✓ 日本語",
            "%already%20encoded",
            "100%",
        ] {
            // `+` is encoded as %2B, so it survives decoding's plus-to-space rule.
            assert_eq!(url_decode(&url_encode(text)).as_deref(), Some(text));
        }
    }

    #[test]
    fn epoch_seconds_become_iso_dates() {
        let answers = rows("ts 1700000000");
        assert_eq!(answers[0].copy, "2023-11-14T22:13:20Z");
        assert_eq!(answers[0].label, "Unix seconds to ISO 8601 (UTC)");
        assert_eq!(answers[1].label, "Relative");
        assert_eq!(answers[1].copy, "just now");
        assert_eq!(copy("ts 0"), "1970-01-01T00:00:00Z");
        assert_eq!(copy("ts 951782400"), "2000-02-29T00:00:00Z");
        assert_eq!(copy("ts -1"), "1969-12-31T23:59:59Z");
        assert_eq!(copy("ts -86400"), "1969-12-31T00:00:00Z");
        assert_eq!(copy("ts 4102444800"), "2100-01-01T00:00:00Z");
        assert_eq!(copy("timestamp 86400"), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn epoch_milliseconds_are_detected_by_magnitude() {
        let answers = rows("ts 1700000000123");
        assert_eq!(answers[0].copy, "2023-11-14T22:13:20.123Z");
        assert_eq!(answers[0].label, "Unix milliseconds to ISO 8601 (UTC)");
        assert_eq!(copy("ts 99999999999"), "5138-11-16T09:46:39Z");
        assert_eq!(copy("ts 100000000000"), "1973-03-03T09:46:40Z");
        assert_eq!(copy("ts -1000"), "1969-12-31T23:43:20Z");
        assert_eq!(copy("ts 1700000000.5"), "2023-11-14T22:13:20.500Z");
        assert_eq!(copy("ts -1.5"), "1969-12-31T23:59:58.500Z");
    }

    #[test]
    fn relative_times_use_the_injected_clock() {
        let relative_of = |query: &str| rows(query).remove(1).copy;
        assert_eq!(relative_of("ts 1699999000"), "16 minutes ago");
        assert_eq!(relative_of("ts 1700003600"), "in 1 hour");
        assert_eq!(relative_of("ts 1699913600"), "1 day ago");
        assert_eq!(relative_of("ts 1700000030"), "in 30 seconds");
        assert_eq!(relative_of("ts 1600000000"), "3 years ago");
        assert_eq!(relative_of("ts 1690000000"), "3 months ago");
        assert_eq!(relative_of("ts 1700000002"), "just now");
    }

    #[test]
    fn dates_become_unix_time() {
        let answers = rows("ts 2023-11-14T22:13:20Z");
        assert_eq!(answers[0].label, "Unix seconds");
        assert_eq!(answers[0].copy, "1700000000");
        assert_eq!(answers[1].label, "Unix milliseconds");
        assert_eq!(answers[1].copy, "1700000000000");
        assert_eq!(copy("ts 2023-11-14"), "1699920000");
        assert_eq!(copy("ts 2023-11-14 22:13:20"), "1700000000");
        assert_eq!(copy("ts 2023-11-14T22:13"), "1699999980");
        assert_eq!(copy("ts 1970-01-01"), "0");
        assert_eq!(copy("ts 1969-12-31T23:59:59Z"), "-1");
        assert_eq!(copy("ts 2023-11-14t22:13:20z"), "1700000000");
        assert_eq!(rows("ts 2023-11-14T22:13:20.250Z")[1].copy, "1700000000250");
        assert_eq!(rows("ts 2023-11-14T22:13:20.5Z")[1].copy, "1700000000500");
    }

    #[test]
    fn dates_honour_utc_offsets() {
        assert_eq!(copy("ts 2023-11-15T00:13:20+02:00"), "1700000000");
        assert_eq!(copy("ts 2023-11-15T00:13:20+0200"), "1700000000");
        assert_eq!(copy("ts 2023-11-15T00:13:20+02"), "1700000000");
        assert_eq!(copy("ts 2023-11-14T17:13:20-05:00"), "1700000000");
        assert_eq!(copy("ts 2023-11-14 17:13:20 -05:00"), "1700000000");
        assert_eq!(copy("ts 2023-11-14T22:43:20+00:30"), "1700000000");
    }

    #[test]
    fn dates_round_trip_through_epoch_conversion() {
        for seconds in [
            0i64,
            1,
            86_399,
            86_400,
            951_782_400,
            1_700_000_000,
            4_102_444_800,
            -1,
            -2_208_988_800,
            253_402_300_799,
        ] {
            let iso = iso_utc(seconds * 1000);
            assert_eq!(parse_iso(&iso), Some(seconds * 1000), "{iso}");
        }
        for days in [-800_000i64, -1, 0, 1, 11_017, 19_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn invalid_timestamps_are_rejected() {
        for query in [
            "ts abc",
            "ts 12abc",
            "ts 1.2.3",
            "ts 2023-13-01",
            "ts 2023-02-29",
            "ts 2023-00-10",
            "ts 2023-11-31",
            "ts 2023-11-14T25:00",
            "ts 2023-11-14T22:60",
            "ts 2023-11-14T22:13:60",
            "ts 2023-11-14T22",
            "ts 2023-11-14X22:13",
            "ts 2023-11-14T22:13:20Zjunk",
            "ts 2023-11-14T22:13:20+25:00",
            "ts 2023-11-14T22:13:20.Z",
            "ts 99999999999999999999",
            "ts 9999999999999999",
            "ts 2023-1-1",
            "ts 0000-01-01",
            "ts 10000-01-01",
            "ts --5",
            "ts -",
            "ts 2023-11-14T22:13:20+02:7",
        ] {
            none(query);
        }
    }

    #[test]
    fn leap_days_are_validated() {
        assert!(evaluate("ts 2024-02-29", &TestEnv::new()).is_some());
        assert!(evaluate("ts 2000-02-29", &TestEnv::new()).is_some());
        none("ts 1900-02-29");
        none("ts 2100-02-29");
    }

    #[test]
    fn ts_alone_and_now_show_the_current_time() {
        for query in ["ts", "ts now", "TS NOW"] {
            let answers = rows(query);
            assert_eq!(answers[0].copy, "1700000000");
            assert_eq!(answers[1].copy, "1700000000000");
            assert_eq!(answers[2].copy, "2023-11-14T22:13:20Z");
        }
    }

    #[test]
    fn json_pretty_prints_and_minifies() {
        let answers = rows(r#"json {"b":1,"a":[1,2,{"c":null}],"d":{}}"#);
        assert_eq!(answers[0].label, "JSON pretty");
        assert_eq!(
            answers[0].copy,
            "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2,\n    {\n      \"c\": null\n    }\n  ],\n  \"d\": {}\n}"
        );
        assert_eq!(answers[1].label, "JSON minified");
        assert_eq!(answers[1].copy, r#"{"b":1,"a":[1,2,{"c":null}],"d":{}}"#);
    }

    #[test]
    fn json_display_is_one_line_and_copy_is_exact() {
        let answers = rows(r#"json {"a":[1,2]}"#);
        assert!(!answers[0].display.contains('\n'));
        assert_eq!(answers[0].display, r#"{ "a": [ 1, 2 ] }"#);
        assert!(answers[0].copy.contains('\n'));
    }

    #[test]
    fn json_keeps_key_order_escapes_numbers_and_unicode() {
        let input = r#"{ "z" : "line\nbreak \" quote \\ ✓", "a" : 12345678901234567890123, "f": 1.50e+3, "é": [ true , false ] }"#;
        let answers = rows(&format!("json {input}"));
        assert_eq!(
            answers[1].copy,
            r#"{"z":"line\nbreak \" quote \\ ✓","a":12345678901234567890123,"f":1.50e+3,"é":[true,false]}"#
        );
        assert!(answers[0].copy.starts_with("{\n  \"z\": \"line\\nbreak"));
        assert!(answers[0].copy.contains("\"a\": 12345678901234567890123,"));
    }

    #[test]
    fn json_keeps_structural_characters_inside_strings() {
        let answers = rows(r#"json ["a, b: {c}", "]\\"]"#);
        assert_eq!(answers[1].copy, r#"["a, b: {c}","]\\"]"#);
        assert_eq!(answers[0].copy, "[\n  \"a, b: {c}\",\n  \"]\\\\\"\n]");
    }

    #[test]
    fn json_handles_scalars_and_empty_containers() {
        assert_eq!(copy("json 42"), "42");
        assert_eq!(copy("json \"hi\""), "\"hi\"");
        assert_eq!(copy("json [ ]"), "[]");
        assert_eq!(copy("json { }"), "{}");
        assert_eq!(copy("json [[],{}]"), "[\n  [],\n  {}\n]");
        assert_eq!(copy("json null"), "null");
    }

    #[test]
    fn json_is_idempotent_and_valid_after_formatting() {
        let input = r#"{"a":[1,{"b":[ ]}],"c":"x"}"#;
        let pretty = reformat_json(input, true);
        assert_eq!(reformat_json(&pretty, true), pretty);
        assert_eq!(reformat_json(&pretty, false), input.replace("[ ]", "[]"));
        assert!(serde_json::from_str::<serde_json::Value>(&pretty).is_ok());
    }

    #[test]
    fn invalid_json_is_rejected() {
        for query in [
            "json {",
            "json {\"a\":}",
            "json {'a':1}",
            "json [1,]",
            "json {\"a\":1,}",
            "json hello",
            "json {\"a\":1} extra",
            "json [1 2]",
            "json {a:1}",
            "json \"unterminated",
            "json {\"a\":01}",
        ] {
            none(query);
        }
    }

    #[test]
    fn case_conversions() {
        assert_eq!(copy("snake Hello World"), "hello_world");
        assert_eq!(copy("kebab Hello World"), "hello-world");
        assert_eq!(copy("camel hello world"), "helloWorld");
        assert_eq!(copy("pascal hello world"), "HelloWorld");
        assert_eq!(copy("upper hello World"), "HELLO WORLD");
        assert_eq!(copy("lower Hello WORLD"), "hello world");
        assert_eq!(copy("title hello wORLD of rust"), "Hello World Of Rust");
        assert_eq!(rows("snake a b")[0].label, "snake_case");
        assert_eq!(rows("camel a b")[0].label, "camelCase");
        assert_eq!(rows("pascal a b")[0].label, "PascalCase");
        assert_eq!(rows("kebab a b")[0].label, "kebab-case");
        assert_eq!(rows("title a b")[0].label, "Title Case");
    }

    #[test]
    fn word_splitting_handles_existing_cases_acronyms_and_digits() {
        assert_eq!(copy("snake helloWorld"), "hello_world");
        assert_eq!(copy("snake HelloWorld"), "hello_world");
        assert_eq!(copy("snake hello-world_foo bar"), "hello_world_foo_bar");
        assert_eq!(copy("snake HTTPServerError"), "http_server_error");
        assert_eq!(copy("kebab getHTTPResponse2"), "get-http-response2");
        assert_eq!(copy("camel SCREAMING_SNAKE_CASE"), "screamingSnakeCase");
        assert_eq!(copy("pascal foo2bar baz"), "Foo2barBaz");
        assert_eq!(copy("camel  spaced   out "), "spacedOut");
        assert_eq!(
            copy("snake __leading_and_trailing__"),
            "leading_and_trailing"
        );
        assert_eq!(copy("kebab version 2 beta"), "version-2-beta");
    }

    #[test]
    fn case_conversions_handle_unicode() {
        assert_eq!(copy("upper straße"), "STRASSE");
        assert_eq!(copy("lower ÉCOLE"), "école");
        assert_eq!(copy("title élan vital ÜBER"), "Élan Vital Über");
        assert_eq!(copy("snake CaféÉlan"), "café_élan");
        assert_eq!(copy("pascal naïve façade"), "NaïveFaçade");
        assert_eq!(copy("snake 日本語 テスト"), "日本語_テスト");
    }

    #[test]
    fn case_conversions_without_words_are_rejected() {
        for query in [
            "snake ---",
            "camel !!!",
            "kebab ___",
            "pascal ...",
            "snake",
            "title",
        ] {
            none(query);
        }
    }

    #[test]
    fn passwords_have_the_requested_length_and_every_class() {
        let env = TestEnv::new();
        for query in ["pw", "pw 4", "pw 8", "pw 20", "pw 128"] {
            let answers = evaluate(query, &env).unwrap();
            let length = query.strip_prefix("pw ").map_or(20, |n| n.parse().unwrap());
            assert_eq!(answers[0].label, format!("Password ({length})"));
            assert_eq!(
                answers[1].label,
                format!("Alphanumeric password ({length})")
            );
            let full = &answers[0].copy;
            assert_eq!(full.chars().count(), length);
            assert!(full.chars().any(|c| c.is_ascii_lowercase()));
            assert!(full.chars().any(|c| c.is_ascii_uppercase()));
            assert!(full.chars().any(|c| c.is_ascii_digit()));
            assert!(full.chars().any(|c| SYMBOLS.contains(c)));
            let plain = &answers[1].copy;
            assert_eq!(plain.chars().count(), length);
            assert!(plain.chars().all(|c| c.is_ascii_alphanumeric()));
            assert!(plain.chars().any(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn passwords_are_deterministic_per_env_and_differ_between_calls() {
        let first = evaluate("pw 16", &TestEnv::new()).unwrap();
        assert_eq!(evaluate("pw 16", &TestEnv::new()), Some(first.clone()));
        let env = TestEnv::new();
        assert_ne!(evaluate("pw 16", &env), evaluate("pw 16", &env));
    }

    #[test]
    fn password_characters_are_roughly_uniform() {
        let env = TestEnv::new();
        let mut counts = std::collections::HashMap::new();
        for _ in 0..200 {
            for c in generate_password(&env, &[LOWER], 50).unwrap().chars() {
                *counts.entry(c).or_insert(0usize) += 1;
            }
        }
        assert_eq!(counts.len(), 26);
        // 10 000 draws over 26 letters is about 385 each.
        assert!(
            counts.values().all(|&n| (280..500).contains(&n)),
            "{counts:?}"
        );
    }

    #[test]
    fn password_length_must_be_a_number_in_range() {
        for query in [
            "pw 3",
            "pw 0",
            "pw 129",
            "pw -5",
            "pw abc",
            "pw 10 20",
            "pw 1.5",
            "pw 99999999999999999999",
        ] {
            none(query);
        }
    }

    #[test]
    fn random_below_terminates_even_for_a_stuck_generator() {
        struct Stuck;
        impl DevEnv for Stuck {
            fn now_ms(&self) -> u64 {
                0
            }
            fn fill_random(&self, buffer: &mut [u8]) -> bool {
                buffer.fill(0xff);
                true
            }
        }
        assert!(random_below(&Stuck, 7).is_some_and(|n| n < 7));
        assert!(evaluate("pw", &Stuck).is_some());
    }

    #[test]
    fn lorem_gives_words_of_the_classic_passage() {
        assert_eq!(copy("lorem 5"), "Lorem ipsum dolor sit amet.");
        assert_eq!(copy("lorem 1"), "Lorem.");
        assert_eq!(
            copy("lorem 8"),
            "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
        );
        assert_eq!(copy("lorem 9").split_whitespace().count(), 9);
        assert_eq!(copy("lorem").split_whitespace().count(), 50);
        assert_eq!(rows("lorem 3")[0].label, "Lorem ipsum (3 words)");
        assert_eq!(rows("lorem 1")[0].label, "Lorem ipsum (1 word)");
    }

    #[test]
    fn lorem_repeats_the_passage_for_long_requests() {
        let long = copy("lorem 200");
        assert_eq!(long.split_whitespace().count(), 200);
        assert!(long.ends_with('.'));
        assert!(!long.ends_with(",."));
        assert!(long.contains("laborum. Lorem ipsum"));
        assert_eq!(copy("lorem 1000").split_whitespace().count(), 1000);
    }

    #[test]
    fn lorem_count_must_be_a_number_in_range() {
        for query in [
            "lorem 0",
            "lorem 1001",
            "lorem -1",
            "lorem abc",
            "lorem 3 4",
            "lorem 2.5",
        ] {
            none(query);
        }
    }

    #[test]
    fn oversized_input_is_rejected() {
        none(&format!("base64 {}", "x".repeat(MAX_QUERY_BYTES)));
    }

    #[test]
    fn system_env_provides_time_and_randomness() {
        let env = SystemEnv;
        assert!(env.now_ms() > 1_700_000_000_000);
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        assert!(env.fill_random(&mut a) && env.fill_random(&mut b));
        assert_ne!(a, b);
        assert_ne!(a, [0u8; 32]);
        let answers = evaluate("uuid", &env).unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].copy.len(), 36);
    }
}
