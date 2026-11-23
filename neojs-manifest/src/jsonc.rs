//! Strict JSONC parsing for `neo.jsonc`.
//!
//! The accepted dialect is JSON plus two conveniences:
//!
//! - `//` line comments and `/* ... */` block comments, outside strings.
//! - A single trailing comma in an object or array.
//!
//! Comments do not nest. A leading UTF-8 byte-order mark is ignored. Everything
//! else is JSON: single quotes, unquoted keys, `NaN`, `Infinity`, hexadecimal
//! numbers, and multiple top-level values are rejected.
//!
//! Duplicate keys are rejected at every object depth instead of silently taking
//! the last value. `serde_json`'s data model does not retain duplicate keys, so
//! this parser checks before inserting. Objects therefore have one unambiguous
//! value per key.
//!
//! The parser is bounded by [`MAX_JSONC_BYTES`] and [`MAX_JSONC_DEPTH`], so a
//! small input cannot request unbounded work or recurse until the process stack
//! is exhausted. These are parse limits, not a file-size or security boundary
//! for a caller that reads untrusted files.

use serde_json::{Map, Value};

use crate::Error;

/// Maximum bytes accepted by [`parse`].
pub const MAX_JSONC_BYTES: usize = 1024 * 1024;

/// Maximum nesting depth accepted by [`parse`].
///
/// The top-level value has depth zero. A value nested 128 containers deep is
/// the deepest accepted document.
pub const MAX_JSONC_DEPTH: usize = 128;

/// Parses one JSONC document.
///
/// Returns the same [`serde_json::Value`] shape as `serde_json`, with comments
/// removed and trailing commas accepted. A leading byte-order mark is ignored.
/// Object keys are unique at every depth.
///
/// # Errors
///
/// Returns [`Error::Jsonc`] for malformed JSONC, an oversized document, or
/// excessive nesting, and [`Error::DuplicateKey`] for a repeated key.
pub fn parse(text: &str) -> Result<Value, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.len() > MAX_JSONC_BYTES {
        return Err(Error::Jsonc {
            line: 1,
            column: 1,
            message: format!(
                "the document is {} bytes, over the {MAX_JSONC_BYTES}-byte limit",
                text.len()
            ),
        });
    }

    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        position: 0,
    };
    parser.skip_trivia()?;
    if parser.position == parser.bytes.len() {
        return Err(parser.syntax(parser.position, "the document contains no JSON value"));
    }
    let value = parser.parse_value(0)?;
    parser.skip_trivia()?;
    if parser.position != parser.bytes.len() {
        return Err(parser.syntax(
            parser.position,
            "unexpected content after the top-level value",
        ));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_JSONC_DEPTH {
            return Err(self.syntax(
                self.position,
                format!("nesting exceeds the {MAX_JSONC_DEPTH}-level limit"),
            ));
        }
        self.skip_trivia()?;
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b't') => {
                self.parse_literal(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.parse_literal(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'n') => {
                self.parse_literal(b"null")?;
                Ok(Value::Null)
            }
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.parse_number(),
            Some(_) | None => Err(self.syntax(self.position, "expected a JSON value")),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value, Error> {
        self.position += 1;
        let mut object = Map::new();
        self.skip_trivia()?;
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Value::Object(object));
        }

        loop {
            self.skip_trivia()?;
            if self.peek() != Some(b'"') {
                return Err(self.syntax(self.position, "expected a quoted object key"));
            }
            let key_position = self.position;
            let key = self.parse_string()?;
            self.skip_trivia()?;
            if self.peek() != Some(b':') {
                return Err(self.syntax(self.position, "expected `:` after an object key"));
            }
            self.position += 1;
            let value = self.parse_value(depth + 1)?;
            if object.insert(key, value).is_some() {
                let (line, column) = line_and_column(self.text, key_position);
                return Err(Error::DuplicateKey { line, column });
            }
            self.skip_trivia()?;
            match self.peek() {
                Some(b',') => {
                    self.position += 1;
                    self.skip_trivia()?;
                    if self.peek() == Some(b'}') {
                        self.position += 1;
                        break;
                    }
                }
                Some(b'}') => {
                    self.position += 1;
                    break;
                }
                _ => {
                    return Err(self.syntax(self.position, "expected `,` or `}` in an object"));
                }
            }
        }

        Ok(Value::Object(object))
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, Error> {
        self.position += 1;
        let mut array = Vec::new();
        self.skip_trivia()?;
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Value::Array(array));
        }

        loop {
            let value = self.parse_value(depth + 1)?;
            array.push(value);
            self.skip_trivia()?;
            match self.peek() {
                Some(b',') => {
                    self.position += 1;
                    self.skip_trivia()?;
                    if self.peek() == Some(b']') {
                        self.position += 1;
                        break;
                    }
                }
                Some(b']') => {
                    self.position += 1;
                    break;
                }
                _ => {
                    return Err(self.syntax(self.position, "expected `,` or `]` in an array"));
                }
            }
        }

        Ok(Value::Array(array))
    }

    fn parse_string(&mut self) -> Result<String, Error> {
        let start = self.position;
        self.position += 1;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.position += 1;
                    let raw = &self.text[start..self.position];
                    return serde_json::from_str(raw).map_err(|error| {
                        self.syntax(start, format!("invalid string literal: {error}"))
                    });
                }
                b'\\' => {
                    self.position += 1;
                    if self.peek().is_none() {
                        return Err(self.syntax(start, "unterminated string"));
                    }
                    // Skip the escaped byte so an escaped quote does not close
                    // the string. Unicode escapes are validated by serde_json
                    // when the complete literal is parsed.
                    self.position += 1;
                }
                b'\n' | b'\r' => {
                    return Err(
                        self.syntax(self.position, "a string contains an unescaped newline")
                    );
                }
                byte if byte < 0x20 => {
                    return Err(self.syntax(
                        self.position,
                        "a string contains an unescaped control character",
                    ));
                }
                _ => self.position += 1,
            }
        }
        Err(self.syntax(start, "unterminated string"))
    }

    fn parse_number(&mut self) -> Result<Value, Error> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        match self.peek() {
            Some(b'0') => self.position += 1,
            Some(byte) if byte.is_ascii_digit() => {
                self.position += 1;
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            Some(_) | None => {
                return Err(self.syntax(self.position, "expected a digit in a number"));
            }
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            let digits = self.position;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
            }
            if self.position == digits {
                return Err(self.syntax(start, "a number has a decimal point with no digits"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            let digits = self.position;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
            }
            if self.position == digits {
                return Err(self.syntax(start, "a number has an exponent with no digits"));
            }
        }

        let raw = &self.text[start..self.position];
        serde_json::from_str(raw)
            .map_err(|error| self.syntax(start, format!("invalid number: {error}")))
    }

    fn parse_literal(&mut self, literal: &[u8]) -> Result<(), Error> {
        if self.remaining().starts_with(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(self.syntax(
                self.position,
                format!("expected `{}`", String::from_utf8_lossy(literal)),
            ))
        }
    }

    fn skip_trivia(&mut self) -> Result<(), Error> {
        loop {
            while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
                self.position += 1;
            }
            if self.remaining().starts_with(b"//") {
                self.position += 2;
                while let Some(byte) = self.peek() {
                    self.position += 1;
                    if byte == b'\n' {
                        break;
                    }
                }
            } else if self.remaining().starts_with(b"/*") {
                let start = self.position;
                self.position += 2;
                loop {
                    if self.remaining().starts_with(b"*/") {
                        self.position += 2;
                        break;
                    }
                    if self.peek().is_none() {
                        return Err(self.syntax(start, "unterminated block comment"));
                    }
                    self.position += 1;
                }
            } else {
                return Ok(());
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn remaining(&self) -> &[u8] {
        &self.bytes[self.position..]
    }

    fn syntax(&self, position: usize, message: impl Into<String>) -> Error {
        let (line, column) = line_and_column(self.text, position);
        Error::Jsonc {
            line,
            column,
            message: message.into(),
        }
    }
}

/// Returns a one-based line and byte column for a byte offset.
///
/// Columns count bytes rather than Unicode scalar values so the numbers stay
/// cheap to compute and match the byte-oriented scanner. Errors do not echo the
/// surrounding text.
fn line_and_column(text: &str, position: usize) -> (usize, usize) {
    let position = position.min(text.len());
    let mut line = 1;
    let mut line_start = 0;
    for (index, byte) in text.as_bytes()[..position].iter().enumerate() {
        if *byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    (line, position - line_start + 1)
}

#[cfg(test)]
mod tests {
    // Unit tests: the JSONC parser is a critical parser where exact token and
    // boundary assertions are the point. Public manifest behaviour is covered
    // by the integration tests.

    use super::*;

    #[test]
    fn accepts_comments_and_trailing_commas() {
        let value = parse(
            r#"
            // a line comment
            {
                "name": "@scope/pkg", /* a block comment */
                "authors": ["A",],
            }
            "#,
        )
        .expect("valid JSONC");
        assert_eq!(value["name"], "@scope/pkg");
        assert_eq!(value["authors"][0], "A");
    }

    #[test]
    fn ignores_comment_markers_inside_strings() {
        let value = parse(r#"{"a":"// not a comment","b":"/* not one */"}"#)
            .expect("strings protect comment markers");
        assert_eq!(value["a"], "// not a comment");
        assert_eq!(value["b"], "/* not one */");
    }

    #[test]
    fn accepts_a_leading_byte_order_mark() {
        let value = parse("\u{feff}{\"a\":1}").expect("BOM");
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth_without_echoing_the_key() {
        let error = parse(r#"{"secret-key-123":{"b":1,"b":2}}"#).expect_err("duplicate");
        assert!(matches!(error, Error::DuplicateKey { .. }));
        assert!(!error.to_string().contains("secret-key-123"), "{error}");
        assert!(!error.to_string().contains("\"b\""), "{error}");
    }

    #[test]
    fn rejects_unterminated_block_comments() {
        let error = parse("{\"a\":1 /* open").expect_err("unterminated comment");
        assert!(error.to_string().contains("unterminated block comment"));
    }

    #[test]
    fn rejects_a_comma_with_no_following_item() {
        let error = parse("{\"a\":1,,\"b\":2}").expect_err("double comma");
        assert!(error.to_string().contains("expected a quoted object key"));
    }

    #[test]
    fn rejects_leading_zeroes_and_non_json_literals() {
        assert!(parse("{\"a\":01}").is_err());
        assert!(parse("{\"a\":NaN}").is_err());
        assert!(parse("{\"a\":Infinity}").is_err());
    }

    #[test]
    fn rejects_trailing_content() {
        let error = parse("{} {}").expect_err("two values");
        assert!(error.to_string().contains("unexpected content"));
    }

    #[test]
    fn rejects_excessive_nesting() {
        let text = format!(
            "{}0{}",
            "[".repeat(MAX_JSONC_DEPTH + 2),
            "]".repeat(MAX_JSONC_DEPTH + 2)
        );
        let error = parse(&text).expect_err("deep nesting");
        assert!(error.to_string().contains("nesting exceeds"));
    }

    #[test]
    fn line_and_column_are_one_based() {
        let error = parse("{\n  \"a\": ]\n}").expect_err("bad value");
        match error {
            Error::Jsonc { line, column, .. } => {
                assert_eq!(line, 2);
                assert_eq!(column, 8);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
