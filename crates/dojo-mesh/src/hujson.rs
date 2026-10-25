//! Tailscale's policy file is HuJSON: JSON with comments and trailing commas. The validation
//! endpoint takes plain JSON, so a candidate is reduced to JSON before it is checked. The text that
//! is finally sent to replace the policy stays as written, comments included.

/// Removes comments and trailing commas, leaving valid JSON when the input was valid HuJSON.
pub fn standardize(text: &str) -> Result<String, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                out.push('"');
                i += 1;
                loop {
                    let Some(&c) = chars.get(i) else { return Err("a string is never closed".into()) };
                    out.push(c);
                    i += 1;
                    match c {
                        '\\' => {
                            let Some(&escaped) = chars.get(i) else { return Err("a string ends in a backslash".into()) };
                            out.push(escaped);
                            i += 1;
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                loop {
                    match (chars.get(i), chars.get(i + 1)) {
                        (Some('*'), Some('/')) => break,
                        (Some(_), _) => i += 1,
                        (None, _) => return Err("a /* comment is never closed".into()),
                    }
                }
                i += 2;
                out.push(' ');
            }
            ',' => {
                let next = chars[i + 1..].iter().position(|c| !c.is_whitespace());
                let trailing = match next {
                    Some(offset) => {
                        let at = i + 1 + offset;
                        matches!(chars[at], ']' | '}')
                            // a comment between the comma and the bracket
                            || (chars[at] == '/' && trailing_after_comments(&chars, at))
                    }
                    None => false,
                };
                if !trailing {
                    out.push(',');
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    serde_json::from_str::<serde_json::Value>(&out).map_err(|error| format!("not valid HuJSON: {error}"))?;
    Ok(out)
}

/// Whether, skipping comments and whitespace from `at`, the next real character closes a container.
fn trailing_after_comments(chars: &[char], mut at: usize) -> bool {
    loop {
        match chars.get(at) {
            Some(c) if c.is_whitespace() => at += 1,
            Some('/') if chars.get(at + 1) == Some(&'/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            Some('/') if chars.get(at + 1) == Some(&'*') => {
                at += 2;
                while at + 1 < chars.len() && !(chars[at] == '*' && chars[at + 1] == '/') {
                    at += 1;
                }
                at += 2;
            }
            Some(']') | Some('}') => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> serde_json::Value {
        serde_json::from_str(&standardize(text).unwrap()).unwrap()
    }

    #[test]
    fn strips_line_and_block_comments() {
        let text = "{\n  // who may reach whom\n  \"acls\": [ /* none yet */ ],\n  \"x\": 1 // trailing\n}";
        assert_eq!(value(text), serde_json::json!({"acls": [], "x": 1}));
    }

    #[test]
    fn strips_trailing_commas_even_across_comments() {
        assert_eq!(value("{\"a\": [1, 2,], \"b\": {\"c\": 3,},}"), serde_json::json!({"a": [1, 2], "b": {"c": 3}}));
        assert_eq!(value("[1, // one\n 2, /* two */ ]"), serde_json::json!([1, 2]));
    }

    #[test]
    fn leaves_strings_alone() {
        let text = r#"{"url": "http://example.com/a,b", "slash": "a//b /* not a comment */", "q": "say \"hi\", // ok", "comma": ",}"}"#;
        assert_eq!(
            value(text),
            serde_json::json!({"url": "http://example.com/a,b", "slash": "a//b /* not a comment */", "q": "say \"hi\", // ok", "comma": ",}"})
        );
    }

    #[test]
    fn plain_json_passes_through_unchanged_in_meaning() {
        assert_eq!(value("{\"a\": [true, null, 1.5]}"), serde_json::json!({"a": [true, null, 1.5]}));
    }

    #[test]
    fn rejects_text_that_is_not_hujson() {
        for text in ["{\"a\": }", "{\"a\": 1", "{\"a\": \"open}", "/* open", "{'a': 1}"] {
            assert!(standardize(text).is_err(), "{text}");
        }
    }
}
