//! The document's elements, nested, with damage repaired: an end tag
//! closes the elements left open inside it, a stray one is ignored, the
//! end of the document closes everything still open, and elements nested
//! too deep are skipped. Each repair is reported.

use std::borrow::Cow;

use super::lexer::{Kind, Lexer};
use super::Problems;
use crate::limits::MAX_DEPTH;

/// An element and what it holds.
pub(super) struct Element<'a> {
    pub(super) name: &'a str,
    /// Where its start tag is.
    pub(super) at: usize,
    pub(super) children: Vec<Node<'a>>,
}

pub(super) enum Node<'a> {
    Element(Element<'a>),
    Text(Cow<'a, str>),
}

/// The document's root element, if it has one.
pub(super) fn build<'a>(text: &'a str, problems: &mut Problems) -> Option<Element<'a>> {
    let mut tree = Builder::default();
    let tokens: Vec<_> = Lexer::new(text, problems).collect();
    for token in tokens {
        match token.kind {
            Kind::Open { name, empty } => tree.open(name, empty, token.at, problems),
            Kind::Close { name } => tree.close(name, token.at, problems),
            Kind::Text(text) => tree.text(text, token.at, problems),
        }
    }
    tree.finish(problems)
}

#[derive(Default)]
struct Builder<'a> {
    /// The elements open, outermost first.
    open: Vec<Element<'a>>,
    root: Option<Element<'a>>,
    /// Elements open inside one skipped for its depth.
    skipping: usize,
}

impl<'a> Builder<'a> {
    fn open(&mut self, name: &'a str, empty: bool, at: usize, problems: &mut Problems) {
        if self.skipping > 0 {
            self.skipping += usize::from(!empty);
            return;
        }
        if self.open.len() >= MAX_DEPTH {
            problems.push((
                at,
                format!("<{name}> is nested deeper than {MAX_DEPTH} levels, skipped"),
            ));
            self.skipping = usize::from(!empty);
            return;
        }
        let element = Element {
            name,
            at,
            children: Vec::new(),
        };
        if empty {
            self.attach(element, problems);
        } else {
            self.open.push(element);
        }
    }

    fn close(&mut self, name: &str, at: usize, problems: &mut Problems) {
        if self.skipping > 0 {
            self.skipping -= 1;
            return;
        }
        let Some(position) = self.open.iter().rposition(|element| element.name == name) else {
            problems.push((at, format!("an end tag </{name}> closes nothing, ignored")));
            return;
        };
        // The element closed, then those left open inside it.
        let mut closed = self.open.split_off(position);
        while let Some(element) = closed.pop() {
            match closed.last_mut() {
                Some(parent) => {
                    problems.push((
                        element.at,
                        format!("<{}> is closed by </{name}>", element.name),
                    ));
                    parent.children.push(Node::Element(element));
                }
                None => self.attach(element, problems),
            }
        }
    }

    fn text(&mut self, text: Cow<'a, str>, at: usize, problems: &mut Problems) {
        if self.skipping > 0 {
            return;
        }
        match self.open.last_mut() {
            Some(parent) => parent.children.push(Node::Text(text)),
            None if text.trim().is_empty() => {}
            None => problems.push((at, "text outside the root element, ignored".into())),
        }
    }

    /// Puts a complete element in its parent, or makes it the root.
    fn attach(&mut self, element: Element<'a>, problems: &mut Problems) {
        if let Some(parent) = self.open.last_mut() {
            parent.children.push(Node::Element(element));
        } else if self.root.is_none() {
            self.root = Some(element);
        } else {
            problems.push((
                element.at,
                format!("a second root element <{}>, ignored", element.name),
            ));
        }
    }

    fn finish(mut self, problems: &mut Problems) -> Option<Element<'a>> {
        while let Some(element) = self.open.pop() {
            problems.push((
                element.at,
                format!("the document ends inside <{}>", element.name),
            ));
            self.attach(element, problems);
        }
        self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree as compact text: elements as `name(children)`, text as is.
    fn outline(element: &Element) -> String {
        let children: Vec<String> = element
            .children
            .iter()
            .map(|child| match child {
                Node::Element(element) => outline(element),
                Node::Text(text) => text.to_string(),
            })
            .collect();
        format!("{}({})", element.name, children.join(","))
    }

    fn tree(text: &str) -> (String, usize) {
        let mut problems = Problems::new();
        let root = build(text, &mut problems).map_or_else(String::new, |root| outline(&root));
        (root, problems.len())
    }

    #[test]
    fn a_well_formed_document_needs_no_repair() {
        assert_eq!(
            tree("<plist><dict><key>a</key><true/></dict></plist>"),
            ("plist(dict(key(a),true()))".into(), 0)
        );
    }

    #[test]
    fn unclosed_and_stray_tags_are_repaired() {
        assert_eq!(
            tree("<plist><array><string>x</array></b></plist>"),
            ("plist(array(string(x)))".into(), 2)
        );
        assert_eq!(
            tree("<plist><array><true/>"),
            ("plist(array(true()))".into(), 2)
        );
        assert_eq!(tree("<a/><b/>x"), ("a()".into(), 2));
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = "<array>".repeat(MAX_DEPTH + 10) + &"</array>".repeat(MAX_DEPTH + 10);
        let (outline, problems) = tree(&deep);
        assert_eq!(outline.matches("array").count(), MAX_DEPTH);
        assert_eq!(problems, 1);
    }
}
