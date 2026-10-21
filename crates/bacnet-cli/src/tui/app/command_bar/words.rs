//! The command bar's words (#1800). The shell that once read these lines is
//! gone, so the bar splits them itself, by a few rules:
//!
//! - Whitespace outside quotes separates words.
//! - Single or double quotes group what is between them, spaces included,
//!   into the word; the other kind of quote is plain text inside them.
//!   Quoted and unquoted parts next to each other make one word, so
//!   `"a"'b'c` is `abc`, and `""` is an empty word.
//! - A backslash, inside quotes or out, makes the quote, whitespace
//!   character or backslash after it plain text. Before anything else, or
//!   at the end of the line, it is a backslash.
//! - A quote left open is an error, not a guess.

/// One word of a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    /// The word with its quotes and escapes taken out.
    pub(crate) text: String,
    /// When part of the word was quoted or escaped, where in `text` the
    /// last such part ends: a quoted or escaped value is a CharacterString,
    /// and an `@P` after this point is its priority.
    pub(crate) literal_to: Option<usize>,
}

/// Split `line` into words, or say which quote is left open.
pub(crate) fn split(line: &str) -> Result<Vec<Word>, String> {
    let mut words = Vec::new();
    let mut word: Option<Word> = None;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        let current = word.get_or_insert_with(|| Word {
            text: String::new(),
            literal_to: None,
        });
        match ch {
            '\\' => match chars.peek() {
                Some(&next) if matches!(next, '\'' | '"' | '\\') || next.is_whitespace() => {
                    current.text.push(next);
                    current.literal_to = Some(current.text.len());
                    chars.next();
                }
                _ => current.text.push('\\'),
            },
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            _ if quote == Some(ch) => {
                quote = None;
                current.literal_to = Some(current.text.len());
            }
            _ if quote.is_none() && ch.is_whitespace() => {
                // `word` was just made for this character: drop it unless
                // it held something before.
                let ended = word
                    .take()
                    .filter(|w| !w.text.is_empty() || w.literal_to.is_some());
                words.extend(ended);
            }
            _ => current.text.push(ch),
        }
    }
    if let Some(open) = quote {
        return Err(format!(
            "unclosed {open} quote: close it, or write \\{open} for the character"
        ));
    }
    words.extend(word.filter(|w| !w.text.is_empty() || w.literal_to.is_some()));
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &str) -> Vec<String> {
        split(line)
            .unwrap_or_else(|e| panic!("{line}: {e}"))
            .into_iter()
            .map(|w| w.text)
            .collect()
    }

    #[test]
    fn whitespace_separates_words() {
        assert_eq!(texts("  watch\t100  ai:1 "), ["watch", "100", "ai:1"]);
        assert!(texts("").is_empty());
        assert!(texts("   ").is_empty());
    }

    #[test]
    fn quotes_group_words() {
        assert_eq!(texts(r#"a "two words" c"#), ["a", "two words", "c"]);
        assert_eq!(texts("a 'two  words'"), ["a", "two  words"]);
        // The other quote is plain inside.
        assert_eq!(texts(r#"'say "hi"' "it's""#), [r#"say "hi""#, "it's"]);
        // Adjacent parts are one word.
        assert_eq!(texts(r#""a"'b'c d"#), ["abc", "d"]);
        assert_eq!(texts(r#"x"y z"w"#), ["xy zw"]);
        // An empty quoted string is an empty word, wherever it is.
        assert_eq!(texts(r#""" '' x"#), ["", "", "x"]);
        assert_eq!(texts(r#"a """#), ["a", ""]);
    }

    #[test]
    fn backslash_escapes_a_quote_a_space_or_a_backslash() {
        assert_eq!(texts(r"two\ words"), ["two words"]);
        assert_eq!(texts(r#"\"x\" \'y"#), [r#""x""#, "'y"]);
        assert_eq!(texts(r"a\\ b"), [r"a\", "b"]);
        assert_eq!(texts(r#""say \"hi\"""#), [r#"say "hi""#]);
        assert_eq!(texts(r"'it\'s'"), ["it's"]);
        // Before anything else, and at the end, it is itself.
        assert_eq!(texts(r"C:\temp \n"), [r"C:\temp", r"\n"]);
        assert_eq!(texts(r"end\"), [r"end\"]);
        assert_eq!(texts(r"\"), [r"\"]);
        // An escaped space starts no new word, and isn't dropped.
        assert_eq!(texts(r"\ "), [" "]);
        // Any whitespace character can be escaped, as any splits words.
        assert_eq!(texts("a\\\tb c\\\u{3000}d"), ["a\tb", "c\u{3000}d"]);
        assert_eq!(texts("x\u{3000}y"), ["x", "y"]);
    }

    #[test]
    fn non_ascii_words_split_and_quote_like_any_other() {
        assert_eq!(texts(r#"say "wörld ü" é\ x"#), ["say", "wörld ü", "é x"]);
        assert_eq!(texts("'日本 語' ß"), ["日本 語", "ß"]);
        let words = split(r"é\ x").unwrap();
        assert_eq!(words[0].literal_to, Some("é ".len()));
    }

    #[test]
    fn an_open_quote_is_an_error() {
        let error = split(r#"writem 100 av:1 description "two words"#).unwrap_err();
        assert!(error.contains("unclosed \" quote"), "{error}");
        assert!(split("x 'y").unwrap_err().contains("unclosed ' quote"));
        // A backslash can't close one: it escapes the quote.
        assert!(split(r#""abc\""#).is_err());
        assert!(split(r#"'"'"#).is_ok());
    }

    #[test]
    fn a_word_knows_where_its_quoted_or_escaped_part_ends() {
        let words = split(r#"plain "q"@8 "a@b"@3 a"b"c \"e\" Zone\ 2@9 C:\temp"#).unwrap();
        let ends: Vec<_> = words.iter().map(|w| w.literal_to).collect();
        assert_eq!(
            ends,
            [None, Some(1), Some(3), Some(2), Some(3), Some(5), None]
        );
        assert_eq!(words[1].text, "q@8");
        assert_eq!(words[2].text, "a@b@3");
        assert_eq!(words[5].text, "Zone 2@9");
    }
}
