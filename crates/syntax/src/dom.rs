//! A bounded XML tree shared by the syntax readers.
//!
//! Readers need elements, attributes, and text, and nothing else, so the tree is
//! built by hand rather than through a DOM library. Documents come from third
//! parties: a DTD is rejected outright (so no entity expansion), and nesting
//! depth is bounded here while the size is bounded by the caller.

use crux::{Currency, Money};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::read::ImportError;

/// Deepest element nesting a reader accepts.
pub(crate) const MAX_DEPTH: usize = 64;

/// One element, with its namespace prefix removed from the name.
pub(crate) struct Node {
    /// Local name, e.g. `Invoice` for `ubl:Invoice`.
    pub(crate) name: String,
    /// Name as written, prefix included.
    pub(crate) raw_name: String,
    attributes: Vec<(String, String)>,
    pub(crate) children: Vec<Node>,
    pub(crate) text: String,
}

impl Node {
    /// First child with this local name.
    pub(crate) fn first(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child with this local name, in document order.
    pub(crate) fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// Attribute value, matched on the name as written (prefix included).
    pub(crate) fn attribute(&self, name: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }
}

/// Trimmed, non-empty text of the first child with this local name.
pub(crate) fn text(node: &Node, name: &str) -> Option<String> {
    node.first(name)
        .map(|child| child.text.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Amount in the document currency, with the element named in the error.
pub(crate) fn money(
    node: Option<&Node>,
    currency: Currency,
    field: &'static str,
) -> Result<Option<Money>, ImportError> {
    let Some(node) = node else {
        return Ok(None);
    };
    let value = node.text.trim();
    Money::parse_fixed(value, currency)
        .map(Some)
        .map_err(|error| ImportError::Invalid {
            field,
            value: format!("{value} ({error})"),
        })
}

/// Namespace of the root element, from its own declaration.
pub(crate) fn namespace(root: &Node) -> Option<String> {
    match root.raw_name.split_once(':') {
        Some((prefix, _)) => root.attribute(&format!("xmlns:{prefix}")),
        None => root.attribute("xmlns"),
    }
}

/// Parses well-formed XML into a bounded tree of local names.
pub(crate) fn parse(xml: &str) -> Result<Node, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| ImportError::NotXml(error.to_string()))?;
        match event {
            Event::Start(element) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(ImportError::TooDeep);
                }
                let raw_name = element.name().as_ref().to_owned();
                stack.push(Node {
                    name: local(element.name().as_ref()),
                    raw_name,
                    attributes: attributes(&element),
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            Event::Empty(element) => {
                let raw_name = element.name().as_ref().to_owned();
                let node = Node {
                    name: local(element.name().as_ref()),
                    raw_name,
                    attributes: attributes(&element),
                    children: Vec::new(),
                    text: String::new(),
                };
                attach(&mut stack, &mut root, node)?;
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| ImportError::NotXml("unbalanced element".to_owned()))?;
                attach(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                if let Some(current) = stack.last_mut() {
                    let value = text.into_inner().into_owned();
                    push_text(&mut current.text, &value);
                }
            }
            Event::CData(data) => {
                if let Some(current) = stack.last_mut() {
                    let value: &str = data.as_ref();
                    push_text(&mut current.text, value);
                }
            }
            Event::DocType(_) => return Err(ImportError::Doctype),
            Event::Eof => break,
            _ => {}
        }
    }

    if !stack.is_empty() {
        return Err(ImportError::NotXml("unclosed elements".to_owned()));
    }
    root.ok_or_else(|| ImportError::NotXml("empty document".to_owned()))
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) -> Result<(), ImportError> {
    match stack.last_mut() {
        Some(parent) => {
            parent.children.push(node);
            Ok(())
        }
        None if root.is_none() => {
            *root = Some(node);
            Ok(())
        }
        None => Err(ImportError::NotXml("more than one root element".to_owned())),
    }
}

fn push_text(target: &mut String, value: &str) {
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(value);
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

fn attributes(element: &quick_xml::events::BytesStart<'_>) -> Vec<(String, String)> {
    element
        .attributes()
        .flatten()
        .map(|attribute| {
            // Attribute names keep their prefix: namespace declarations are
            // looked up as `xmlns` / `xmlns:prefix`.
            (attribute.key.as_ref().to_owned(), attribute.value.into_owned())
        })
        .collect()
}
