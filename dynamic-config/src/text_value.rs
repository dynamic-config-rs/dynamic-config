//! What a string means when it arrives as configuration.
//!
//! An environment variable, a `--set` argument, a `.env` line and a bound
//! variable all arrive as text and all have to become a typed value:
//! `8080` is a port, `true` is a flag, `[a, b]` is a list. This module is
//! that reading, and it is the *only* place in the crate that performs it —
//! the four callers share one grammar so `APP_PORT=8080` and
//! `--set port=8080` cannot come to different conclusions.
//!
//! The grammar is deliberately the one this crate has always had, kept
//! bug-for-bug rather than improved: it shipped in every release so far and
//! a configuration that read one way yesterday reads the same way today.
//! `tests/text_value.rs` and `fuzz/fuzz_targets/text_value.rs` hold it to
//! that, comparing against the original implementation for every input they
//! can generate.
//!
//! ```text
//! value  = "true" | "false" | dict | array | string | char | bare
//! dict   = "{" (key "=" value) ("," (key "=" value))* ","? "}"
//! array  = "[" value ("," value)* ","? "]"
//! string = '"' escaped* '"'
//! char   = "'" any "'"
//! bare   = anything up to , { } [ ] — trimmed, then read as a number if it
//!          reads as one, and kept as text if it does not
//! ```
//!
//! Anything the grammar refuses is text: a value that does not parse is not
//! an error, it is a string. That rule is why a password with a `{` in it
//! survives.

use std::collections::BTreeMap;

use crate::value::Value;

/// Reads `text` the way this crate reads configuration text.
///
/// Never fails: input the grammar cannot read becomes
/// [`Value::String`] holding the original, untouched.
pub(crate) fn from_text(text: &str) -> Value {
    let mut reader = Reader { input: text, at: 0 };

    match reader.value() {
        // The whole input, or nothing: a trailing `x` after `true` means the
        // text was never a boolean, so it is the string it always was.
        Some(value) if reader.rest().is_empty() => value,
        _ => Value::String(text.to_owned()),
    }
}

/// A cursor over the text. Every method returns `None` for "this is not what
/// I parse", and the caller turns that into the string fallback.
struct Reader<'a> {
    input: &'a str,
    at: usize,
}

impl<'a> Reader<'a> {
    fn rest(&self) -> &'a str {
        &self.input[self.at..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn eat(&mut self, expected: char) -> Option<()> {
        let next = self.peek()?;

        if next == expected {
            self.at += next.len_utf8();
            Some(())
        } else {
            None
        }
    }

    fn eat_any(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.at += next.len_utf8();

        Some(next)
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.rest().starts_with(word) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(next) = self.peek() {
            if next.is_ascii_whitespace() {
                self.at += next.len_utf8();
            } else {
                break;
            }
        }
    }

    fn take_while(&mut self, mut wanted: impl FnMut(char) -> bool) -> &'a str {
        let start = self.at;

        while let Some(next) = self.peek() {
            if wanted(next) {
                self.at += next.len_utf8();
            } else {
                break;
            }
        }

        &self.input[start..self.at]
    }

    /// One value, surrounded by whatever whitespace it likes.
    fn value(&mut self) -> Option<Value> {
        self.skip_whitespace();

        let value = if self.eat_word("true") {
            Value::Bool(true)
        } else if self.eat_word("false") {
            Value::Bool(false)
        } else {
            match self.peek() {
                Some('{') => Value::Table(self.dict()?),
                Some('[') => Value::Array(self.array()?),
                Some('"') => Value::String(self.string()?),
                // A single quote holds exactly one character — `'a'` is the
                // letter and `'ab'` is not a value at all, which is the
                // original grammar's shape and not a simplification of it.
                Some('\'') => {
                    self.eat('\'')?;
                    let character = self.eat_any()?;
                    self.eat('\'')?;

                    Value::String(character.to_string())
                }
                _ => bare(self.take_while(is_not_separator).trim()),
            }
        };

        self.skip_whitespace();

        Some(value)
    }

    /// `[` value `,` … `]`, a trailing comma allowed and an empty list too.
    fn array(&mut self) -> Option<Vec<Value>> {
        self.delimited('[', ']', |reader| reader.value())
    }

    /// `{` key `=` value `,` … `}`, same rules.
    fn dict(&mut self) -> Option<BTreeMap<String, Value>> {
        let entries = self.delimited('{', '}', |reader| {
            reader.skip_whitespace();
            let key = reader.key()?;
            reader.skip_whitespace();
            reader.eat('=')?;
            let value = reader.value()?;

            Some((key, value))
        })?;

        Some(entries.into_iter().collect())
    }

    /// The shape both collections share: read items until the closing token,
    /// where a separator is optional before it.
    fn delimited<T>(
        &mut self,
        open: char,
        close: char,
        mut item: impl FnMut(&mut Self) -> Option<T>,
    ) -> Option<Vec<T>> {
        self.eat(open)?;

        let mut collected = Vec::new();

        loop {
            if self.eat(close).is_some() {
                return Some(collected);
            }

            collected.push(item(self)?);

            if self.eat(',').is_none() {
                self.eat(close)?;

                return Some(collected);
            }
        }
    }

    /// A quoted key, or a bare one spelled with identifier characters.
    fn key(&mut self) -> Option<String> {
        if self.peek() == Some('"') {
            return self.string();
        }

        let key = self.take_while(is_key_char);

        if key.is_empty() {
            None
        } else {
            Some(key.to_owned())
        }
    }

    /// `"` … `"`, with the escapes a quoted string is allowed to carry.
    fn string(&mut self) -> Option<String> {
        self.eat('"')?;

        let mut escaped = false;
        let inner = self.take_while(|character| {
            if escaped {
                escaped = false;
                return true;
            }

            if character == '\\' {
                escaped = true;
                return true;
            }

            character != '"'
        });

        self.eat('"')?;

        unescape(inner)
    }
}

/// Everything that is not a collection's punctuation. Whitespace is *not* a
/// terminator — `a b` is the string `a b`, which is why the caller trims
/// rather than splits.
fn is_not_separator(character: char) -> bool {
    !matches!(character, ',' | '{' | '}' | '[' | ']')
}

fn is_key_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// Text with no quotes and no punctuation: a number if it reads as one,
/// otherwise the text itself.
///
/// The order matters and is the original's: a float only when a `.` is
/// present, then unsigned, then signed. `1e5` has no `.`, so it is a string —
/// surprising once, and stable for as long as this crate has existed.
fn bare(text: &str) -> Value {
    if text.contains('.') {
        if let Ok(float) = text.parse::<f64>() {
            return Value::Float(float);
        }
    }

    if let Ok(unsigned) = text.parse::<usize>() {
        // `usize`/`isize` rather than the widest integer available: a number
        // above the platform's word size stays text, as it always has.
        return Value::Integer(unsigned as i128);
    }

    if let Ok(signed) = text.parse::<isize>() {
        return Value::Integer(signed as i128);
    }

    Value::String(text.to_owned())
}

/// The escapes a quoted string may contain. `None` when the string holds an
/// escape that is not one — the caller then keeps the whole input as text.
fn unescape(string: &str) -> Option<String> {
    let mut characters = string.chars();
    let mut output = String::with_capacity(string.len());

    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next()? {
                '"' => output.push('"'),
                '\\' => output.push('\\'),
                'b' => output.push('\u{8}'),
                'f' => output.push('\u{c}'),
                'n' => output.push('\n'),
                'r' => output.push('\r'),
                't' => output.push('\t'),
                short @ ('u' | 'U') => {
                    let width = if short == 'u' { 4 } else { 8 };
                    output.push(hex(&mut characters, width)?);
                }
                _ => return None,
            },
            // The set a quoted string may hold literally: tab, and everything
            // printable except DEL.
            '\u{09}' => output.push('\u{09}'),
            printable if printable >= '\u{20}' && printable != '\u{7f}' => output.push(printable),
            _ => return None,
        }
    }

    Some(output)
}

/// `\uXXXX` and `\UXXXXXXXX`, hex digits only, and only if they name a
/// character that exists.
fn hex(characters: &mut std::str::Chars<'_>, width: usize) -> Option<char> {
    let mut digits = String::with_capacity(width);

    for _ in 0..width {
        let digit = characters.next()?;

        if digit.is_ascii_hexdigit() {
            digits.push(digit);
        } else {
            return None;
        }
    }

    char::from_u32(u32::from_str_radix(&digits, 16).ok()?)
}

#[cfg(test)]
mod tests {
    use super::from_text;
    use crate::value::Value;

    /// What the original implementation answers, for the same input.
    ///
    /// The oracle is the library this grammar was ported from, compared
    /// through the same conversion the rest of the crate uses — so the
    /// comparison is on meaning, not on which integer width happened to
    /// carry it.
    ///
    /// `None` where the original *panics*: it indexes a byte range with a
    /// character position while unescaping, so a quoted string holding a
    /// non-ASCII character before an escape takes the process down. That is
    /// not a reading to agree with; the port answers those inputs instead of
    /// aborting, and [`no_input_takes_the_process_down`] pins what it says.
    fn original(text: &str) -> Option<Value> {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let parsed = std::panic::catch_unwind(|| text.parse::<figment::value::Value>());

        std::panic::set_hook(previous);

        parsed.ok().map(|value| {
            crate::backend::figment::from_figment(&value.expect("the original never errors"))
        })
    }

    /// The reading no oracle can be asked for: the original aborts on these,
    /// so the port is the only implementation that has an answer.
    ///
    /// It is the answer the original was reaching for when it indexed itself
    /// off a character boundary — the escape resolved, the text kept.
    #[test]
    fn no_input_takes_the_process_down() {
        for (text, expected) in [
            ("\"é\\n\"", "é\n"),
            ("\"ü\\t\"", "ü\t"),
            ("\"aé\\nb\"", "aé\nb"),
            ("\"€\\\\\"", "€\\"),
        ] {
            assert_eq!(
                from_text(text),
                Value::String(expected.to_owned()),
                "{text:?} has a reading, and taking the process down is not it"
            );
        }
    }

    /// The cases the original's own suite pins, restated here so they keep
    /// being pinned on the day the original is no longer in the graph.
    #[test]
    fn the_grammar_reads_what_it_always_read() {
        let cases = [
            "true",
            "false",
            "\"false\"",
            "  false  ",
            "1",
            " -0",
            " -2",
            " 123 ",
            "a ",
            "\" a  \"",
            "1.2",
            "3.14159",
            "\"\\t\"",
            "\"abc\\td\\n\"",
            "\"\\\"hi\\\"\"",
            "\"hi\\u1234there\"",
            "\"\\\\\"",
            // An unterminated escape is not a string; it is the text itself.
            "\"\\\"",
            "[1,2,3]",
            "{a=b}",
            "{\"a.b.c\"=b}",
            "{a=1,b=hi}",
            "[1,[2],3]",
            "{a=[[-2]]}",
            "[1,true,hi,\"a \"]",
            "[1,{a=b},hi]",
            "[[ -1], {a=[ b ]},  hi ]",
            // The corners the grammar is asked about most often.
            "",
            "   ",
            "8080",
            "18446744073709551615",
            "99999999999999999999999999",
            "1e5",
            "1.2.3",
            "'a'",
            "'ab'",
            "truex",
            "[]",
            "{}",
            "[1,]",
            "{a=b,}",
            "[1 2]",
            "postgres://user:pass@host:5432/db",
            "a,b,c",
        ];

        for case in cases {
            assert_eq!(
                Some(from_text(case)),
                original(case),
                "reading {case:?} moved away from the original"
            );
        }
    }

    proptest::proptest! {
        /// Any string at all: most are text, and the ones that are not must
        /// still agree.
        #[test]
        fn every_string_reads_the_same(text in ".*") {
            // A reading the original cannot produce without aborting is not
            // a disagreement — it is the bug this port does not carry over.
            if let Some(expected) = original(&text) {
                proptest::prop_assert_eq!(from_text(&text), expected);
            }
        }

        /// Text shaped like configuration, so the agreement is tested on the
        /// paths that parse rather than only on the fallback.
        #[test]
        fn configuration_shaped_text_reads_the_same(
            text in r#"[ ]*(true|false|-?[0-9]{1,20}|-?[0-9]{1,5}\.[0-9]{1,5}|[a-z]{1,4}|"[a-z ]{0,4}"|'[a-z]'|\[[^]]{0,12}\]|\{[a-z]{1,3}=[a-z0-9]{1,3}\})[ ]*"#
        ) {
            if let Some(expected) = original(&text) {
                proptest::prop_assert_eq!(from_text(&text), expected);
            }
        }
    }
}
