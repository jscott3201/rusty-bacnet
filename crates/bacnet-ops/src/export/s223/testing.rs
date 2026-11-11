//! A small structural check of the Turtle the export writes, by hand so
//! no crate needs an RDF library: the writer's own tests, and front ends'
//! tests (the MCP tool's), run their output through it. Test support, not
//! a stable API (the `test-support` feature).

use std::collections::BTreeSet;

/// A token of Turtle as the check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Prefix,
    Iri(String),
    Literal(String),
    Name(String),
    Datatype,
    Punct(char),
}

/// The tokens of `text`, comments dropped. Panics on a literal that isn't
/// escaped as the writer escapes (no raw control or non-ASCII character,
/// only the escapes Turtle defines), or on a character outside printable
/// ASCII anywhere else.
fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\n' => {}
            '#' => {
                for ch in chars.by_ref() {
                    if ch == '\n' {
                        break;
                    }
                    assert!((' '..='~').contains(&ch), "comment holds {ch:?}");
                }
            }
            '"' => {
                let mut value = String::new();
                loop {
                    let ch = chars.next().expect("unterminated literal");
                    assert!((' '..='~').contains(&ch), "raw {ch:?} in a literal");
                    match ch {
                        '"' => break,
                        '\\' => {
                            let escape = chars.next().expect("escape at the end");
                            let digits = match escape {
                                '"' | '\\' | 'n' | 'r' | 't' => 0,
                                'u' => 4,
                                'U' => 8,
                                other => panic!("escape \\{other}"),
                            };
                            let hex: String = chars.by_ref().take(digits).collect();
                            assert!(
                                hex.len() == digits && hex.chars().all(|c| c.is_ascii_hexdigit()),
                                "\\{escape}{hex}"
                            );
                            value.push(match escape {
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                'u' | 'U' => char::from_u32(u32::from_str_radix(&hex, 16).unwrap())
                                    .expect("a character"),
                                other => other,
                            });
                        }
                        _ => value.push(ch),
                    }
                }
                out.push(Token::Literal(value));
            }
            '<' => {
                let iri: String = chars.by_ref().take_while(|c| *c != '>').collect();
                assert!(!iri.contains([' ', '<', '"']), "{iri}");
                out.push(Token::Iri(iri));
            }
            ';' | ',' => out.push(Token::Punct(ch)),
            '^' => {
                assert_eq!(chars.next(), Some('^'));
                out.push(Token::Datatype);
            }
            '@' => {
                let word: String =
                    std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_alphabetic())).collect();
                assert_eq!(word, "prefix");
                out.push(Token::Prefix);
            }
            _ => {
                assert!((' '..='~').contains(&ch), "raw {ch:?}");
                let mut name = String::from(ch);
                name.extend(std::iter::from_fn(|| {
                    chars.next_if(|c| !matches!(c, ' ' | '\n' | ';' | ',' | '"' | '<' | '^'))
                }));
                // A local name can't end in a dot: one there ends the
                // statement.
                let ends = name.len() > 1 && name.ends_with('.');
                if ends {
                    name.pop();
                }
                if name == "." {
                    out.push(Token::Punct('.'));
                } else {
                    out.push(Token::Name(name));
                    if ends {
                        out.push(Token::Punct('.'));
                    }
                }
            }
        }
    }
    out
}

/// The shape's pattern for an identifier string, re-implemented: a type
/// name of letters, digits and hyphens, a comma, and an instance with no
/// leading zero, so never 0.
pub fn identifier_ok(text: &str) -> bool {
    let Some((name, instance)) = text.split_once(',') else {
        return false;
    };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && instance.starts_with(|c: char| ('1'..='9').contains(&c))
        && instance.chars().all(|c| c.is_ascii_digit())
}

/// Check `text` as the writer should have written it: every prefix used
/// is declared first, every statement is a subject, then predicate and
/// object pairs split by `;`, ended by `.`, and every identifier string
/// fits the shape's pattern. Returns the statements' subjects.
///
/// # Panics
///
/// On the first thing out of place, saying what.
pub fn check_structure(text: &str) -> Vec<String> {
    let tokens = tokens(text);
    let mut declared = BTreeSet::new();
    let mut subjects = Vec::new();
    let mut at = 0;
    let prefix_of = |name: &str| name.split_once(':').map(|(p, _)| p.to_string());
    while at < tokens.len() {
        if tokens[at] == Token::Prefix {
            let [Token::Name(prefix), Token::Iri(_), Token::Punct('.')] = &tokens[at + 1..at + 4]
            else {
                panic!("bad prefix at {:?}", &tokens[at..]);
            };
            assert!(prefix.ends_with(':'), "{prefix}");
            declared.insert(prefix.trim_end_matches(':').to_string());
            at += 4;
            continue;
        }
        let used = |name: &String| {
            let prefix = prefix_of(name).unwrap_or_else(|| panic!("{name} has no prefix"));
            assert!(declared.contains(&prefix), "{prefix}: not declared");
        };
        let Token::Name(subject) = &tokens[at] else {
            panic!("no subject at {:?}", &tokens[at..]);
        };
        used(subject);
        subjects.push(subject.clone());
        at += 1;
        loop {
            let Some(Token::Name(predicate)) = tokens.get(at) else {
                panic!("no predicate after {subject}");
            };
            if predicate != "a" {
                used(predicate);
            }
            at += 1;
            loop {
                match tokens.get(at) {
                    Some(Token::Name(object)) => used(object),
                    Some(Token::Literal(value)) => {
                        if predicate.ends_with("-identifier")
                            && predicate != "bacnet:property-identifier"
                        {
                            assert!(identifier_ok(value), "{predicate} {value:?}");
                        }
                        if tokens.get(at + 1) == Some(&Token::Datatype) {
                            let Some(Token::Name(datatype)) = tokens.get(at + 2) else {
                                panic!("no datatype after {value:?}");
                            };
                            used(datatype);
                            at += 2;
                        }
                    }
                    other => panic!("no object after {predicate}: {other:?}"),
                }
                at += 1;
                if tokens.get(at) == Some(&Token::Punct(',')) {
                    at += 1;
                    continue;
                }
                break;
            }
            match tokens.get(at) {
                Some(Token::Punct(';')) => at += 1,
                Some(Token::Punct('.')) => {
                    at += 1;
                    break;
                }
                other => panic!("statement on {subject} not ended: {other:?}"),
            }
        }
    }
    subjects
}
