//! The XML a property list is written in, cut into tags and text.
//!
//! Enough of XML for property lists, and tolerant of what isn't: start,
//! end and empty-element tags (attributes skipped), text with the five
//! predefined entities and character references decoded, CDATA sections
//! as text; the XML declaration, processing instructions, comments and the
//! document type declaration (internal subset included) skipped. Malformed
//! markup is reported and read as well as it can be.

use std::borrow::Cow;

use super::Problems;

/// One piece of the document, and the byte offset where it starts.
pub(super) struct Token<'a> {
    pub(super) kind: Kind<'a>,
    pub(super) at: usize,
}

pub(super) enum Kind<'a> {
    /// `<name …>`, or `<name …/>` when `empty`.
    Open { name: &'a str, empty: bool },
    /// `</name>`.
    Close { name: &'a str },
    /// Character data, entities decoded.
    Text(Cow<'a, str>),
}

/// The tokens of a document, in order.
pub(super) struct Lexer<'a, 'p> {
    text: &'a str,
    at: usize,
    problems: &'p mut Problems,
}

impl<'a, 'p> Lexer<'a, 'p> {
    pub(super) fn new(text: &'a str, problems: &'p mut Problems) -> Self {
        Self {
            text,
            at: 0,
            problems,
        }
    }

    fn report(&mut self, at: usize, message: impl Into<String>) {
        self.problems.push((at, message.into()));
    }

    /// The markup at the cursor (which is on a `<`): a token, or `None`
    /// when it is skipped.
    fn markup(&mut self) -> Option<Kind<'a>> {
        let start = self.at;
        let rest = &self.text[start..];
        if rest.starts_with("<?") {
            self.skip_past(start, "?>", "processing instruction");
            None
        } else if rest.starts_with("<!--") {
            self.skip_past(start, "-->", "comment");
            None
        } else if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            Some(Kind::Text(Cow::Borrowed(self.cdata(start, cdata))))
        } else if rest.starts_with("<!") {
            self.skip_declaration(start);
            None
        } else if rest.starts_with("</") {
            self.end_tag(start)
        } else if rest[1..].starts_with(is_name_start) {
            self.start_tag(start)
        } else {
            self.report(start, "a '<' that starts no tag, read as text");
            self.at += 1;
            Some(Kind::Text(Cow::Borrowed("<")))
        }
    }

    /// Moves past the next `end`, or to the end of the document (reported).
    fn skip_past(&mut self, start: usize, end: &str, what: &str) {
        if let Some(found) = self.text[start..].find(end) {
            self.at = start + found + end.len();
        } else {
            self.report(start, format!("unterminated {what}"));
            self.at = self.text.len();
        }
    }

    fn cdata(&mut self, start: usize, body: &'a str) -> &'a str {
        let body_start = self.text.len() - body.len();
        let Some(end) = body.find("]]>") else {
            self.report(start, "unterminated CDATA section");
            self.at = self.text.len();
            return body;
        };
        self.at = body_start + end + 3;
        &body[..end]
    }

    /// `<!DOCTYPE …>` and other declarations: up to the `>` outside quotes
    /// and outside the internal subset's brackets.
    fn skip_declaration(&mut self, start: usize) {
        if let Some(end) = tag_end(&self.text[start..], true) {
            self.at = start + end + 1;
        } else {
            self.report(start, "unterminated declaration");
            self.at = self.text.len();
        }
    }

    fn end_tag(&mut self, start: usize) -> Option<Kind<'a>> {
        let Some(end) = self.text[start..].find('>') else {
            self.report(start, "unterminated end tag");
            self.at = self.text.len();
            return None;
        };
        self.at = start + end + 1;
        let name = self.text[start + 2..start + end].trim();
        Some(Kind::Close { name })
    }

    /// `<name attributes>` or `<name attributes/>`; quoted attribute values
    /// may hold `>`.
    fn start_tag(&mut self, start: usize) -> Option<Kind<'a>> {
        let tag = &self.text[start + 1..];
        let name_len = tag
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(tag.len());
        let name = &tag[..name_len];
        let Some(end) = tag_end(tag, false) else {
            self.report(start, format!("unterminated tag <{name}>"));
            self.at = self.text.len();
            return None;
        };
        self.at = start + 1 + end + 1;
        let empty = tag[..end].trim_end().ends_with('/');
        Some(Kind::Open { name, empty })
    }

    /// Character data up to the next `<`.
    fn text(&mut self) -> Kind<'a> {
        let start = self.at;
        let end = self.text[start..]
            .find('<')
            .map_or(self.text.len(), |found| start + found);
        self.at = end;
        Kind::Text(decode_entities(
            &self.text[start..end],
            start,
            self.problems,
        ))
    }
}

impl<'a> Iterator for Lexer<'a, '_> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        while self.at < self.text.len() {
            let at = self.at;
            let kind = if self.text[at..].starts_with('<') {
                self.markup()
            } else {
                Some(self.text())
            };
            if let Some(kind) = kind {
                return Some(Token { kind, at });
            }
        }
        None
    }
}

/// Where the `>` ending a tag or declaration is: the first outside
/// quotes and, with `brackets`, outside square brackets.
fn tag_end(text: &str, brackets: bool) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (offset, byte) in text.bytes().enumerate() {
        if let Some(open) = quote {
            if byte == open {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'[' if brackets => depth += 1,
            b']' if brackets => depth = depth.saturating_sub(1),
            b'>' if depth == 0 => return Some(offset),
            _ => {}
        }
    }
    None
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

/// The longest entity worth looking for a `;` in: `&#x10FFFF;` and a bit.
const MAX_ENTITY_LEN: usize = 12;

/// `raw` with `&lt;` `&gt;` `&amp;` `&quot;` `&apos;` and `&#…;` decoded.
/// Unknown entities stay as written; references to characters that can't
/// be become U+FFFD. Both are reported, at `at` plus their offset.
fn decode_entities<'a>(raw: &'a str, at: usize, problems: &mut Problems) -> Cow<'a, str> {
    if !raw.contains('&') {
        return Cow::Borrowed(raw);
    }
    let mut text = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        text.push_str(&rest[..amp]);
        let entity_at = at + (raw.len() - rest.len()) + amp;
        rest = &rest[amp..];
        let decoded = rest
            .bytes()
            .take(MAX_ENTITY_LEN)
            .position(|byte| byte == b';')
            .map(|semicolon| (semicolon, entity(&rest[1..semicolon])));
        let Some((semicolon, Some(c))) = decoded else {
            problems.push((entity_at, "an unknown entity, kept as written".into()));
            text.push('&');
            rest = &rest[1..];
            continue;
        };
        if c == char::REPLACEMENT_CHARACTER {
            problems.push((entity_at, "a character reference to no character".into()));
        }
        text.push(c);
        rest = &rest[semicolon + 1..];
    }
    text.push_str(rest);
    Cow::Owned(text)
}

/// The character an entity name (between `&` and `;`) stands for.
fn entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => character_reference(name.strip_prefix('#')?),
    }
}

fn character_reference(number: &str) -> Option<char> {
    let (digits, radix) = match number.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (number, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let code = u32::from_str_radix(digits, radix).ok();
    Some(
        code.filter(|&code| code != 0)
            .and_then(char::from_u32)
            .unwrap_or(char::REPLACEMENT_CHARACTER),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> (Vec<String>, Problems) {
        let mut problems = Problems::new();
        let tokens = Lexer::new(text, &mut problems)
            .map(|token| match token.kind {
                Kind::Open { name, empty: true } => format!("<{name}/>"),
                Kind::Open { name, .. } => format!("<{name}>"),
                Kind::Close { name } => format!("</{name}>"),
                Kind::Text(text) => text.into_owned(),
            })
            .collect();
        (tokens, problems)
    }

    #[test]
    fn markup_that_isnt_content_is_skipped() {
        let document = r#"<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "x.dtd" [ <!ENTITY e "a>b"> ]><!-- a <comment> --><plist version="1.0"><true/></plist>"#;
        let (tokens, problems) = tokens(document);
        assert_eq!(tokens, ["<plist>", "<true/>", "</plist>"]);
        assert!(problems.is_empty());
    }

    #[test]
    fn entities_and_cdata_are_decoded() {
        let (tokens, problems) =
            tokens("<s>&lt;&gt;&amp;&quot;&apos;&#65;&#x1F600;<![CDATA[<&amp;>]]></s>");
        assert_eq!(tokens, ["<s>", "<>&\"'A\u{1F600}", "<&amp;>", "</s>"]);
        assert!(problems.is_empty());
    }

    #[test]
    fn bad_entities_are_reported_and_kept() {
        let (tokens, problems) = tokens("a &nbsp; b &#0; c &#xD800; & d");
        assert_eq!(tokens, ["a &nbsp; b \u{FFFD} c \u{FFFD} & d"]);
        assert_eq!(
            problems.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
            [2, 11, 18, 27]
        );
    }

    #[test]
    fn quoted_attributes_may_hold_brackets() {
        let (tokens, _) = tokens(r#"<plist version="1>0" a='/'>x</plist >"#);
        assert_eq!(tokens, ["<plist>", "x", "</plist>"]);
    }

    #[test]
    fn unterminated_markup_ends_the_document() {
        for document in ["<a", "</a", "<!-- x", "<?xml", "<!DOCTYPE [", "<![CDATA[x"] {
            let (_, problems) = tokens(document);
            assert_eq!(problems.len(), 1, "{document}");
        }
        let (tokens, problems) = tokens("1 < 2");
        assert_eq!(tokens, ["1 ", "<", " 2"]);
        assert_eq!(problems.len(), 1);
    }
}
