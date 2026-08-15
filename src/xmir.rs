//! Reader and writer for XMIR, the XML dialect the EO parser emits.

use quick_xml::Reader;
use quick_xml::events::Event;

/// A parsed XMIR document.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Xmir {
    prologue: Vec<Prologue>,
    root: Element,
}

/// What stands before the root element: the XML declaration and the comment
/// the EO parser puts at the top of every file.
#[derive(Debug, PartialEq, Eq, Clone)]
enum Prologue {
    Declaration(String),
    Comment(String),
}

/// One element of an XMIR document, with its attributes in source order.
#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Element {
    pub(crate) tag: String,
    pub(crate) attributes: Vec<(String, String)>,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<Element>,
}

impl Xmir {
    /// Parse XMIR text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut reader = Reader::from_str(text);
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;
        let mut prologue: Vec<Prologue> = Vec::new();
        loop {
            match reader.read_event() {
                Err(e) => return Err(e.to_string()),
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) => stack.push(element(&e)?),
                Ok(Event::Empty(e)) => {
                    let done = element(&e)?;
                    adopt(&mut stack, &mut root, done)?;
                }
                Ok(Event::End(_)) => {
                    let done = stack.pop().ok_or("unbalanced closing tag")?;
                    adopt(&mut stack, &mut root, done)?;
                }
                Ok(Event::Text(e)) => {
                    let text = e.xml10_content().map_err(|e| e.to_string())?.into_owned();
                    append(&mut stack, &text)?;
                }
                Ok(Event::GeneralRef(e)) => {
                    let name = String::from_utf8_lossy(e.as_ref()).into_owned();
                    append(&mut stack, resolve(&name)?)?;
                }
                Ok(Event::Decl(e)) => {
                    let raw = String::from_utf8_lossy(e.as_ref()).into_owned();
                    prologue.push(Prologue::Declaration(raw));
                }
                Ok(Event::Comment(e)) => {
                    if !stack.is_empty() {
                        return Err("comment inside the tree is not modelled".to_string());
                    }
                    let raw = String::from_utf8_lossy(e.as_ref()).into_owned();
                    prologue.push(Prologue::Comment(raw));
                }
                Ok(Event::CData(_)) => return Err("CDATA is not modelled".to_string()),
                Ok(Event::PI(_)) => {
                    return Err("processing instruction is not modelled".to_string());
                }
                Ok(_) => {}
            }
        }
        root.map(|root| Self { prologue, root })
            .ok_or("no root element".to_string())
    }

    /// The root element of the document.
    pub(crate) fn root(&self) -> &Element {
        &self.root
    }

    /// The same document with another tree under the same prologue.
    pub(crate) fn rebuilt(&self, root: Element) -> Self {
        Self {
            prologue: self.prologue.clone(),
            root,
        }
    }

    /// Render the document back to XMIR text.
    pub fn print(&self) -> String {
        let mut out = String::new();
        for item in &self.prologue {
            match item {
                Prologue::Declaration(raw) => out.push_str(&format!("<?{raw}?>\n")),
                Prologue::Comment(raw) => out.push_str(&format!("<!--{raw}-->\n")),
            }
        }
        write(&self.root, 0, &mut out);
        out
    }
}

/// Add a piece of text to the element being read.
///
/// A run of text is split by the reader at every entity reference, so the
/// pieces have to be joined rather than replace one another.
fn append(stack: &mut [Element], piece: &str) -> Result<(), String> {
    let Some(holder) = stack.last_mut() else {
        if piece.trim().is_empty() {
            return Ok(());
        }
        return Err(format!("text {piece:?} outside of any element"));
    };
    match &mut holder.text {
        Some(text) => text.push_str(piece),
        None => holder.text = Some(piece.to_string()),
    }
    Ok(())
}

/// The five entities XML predefines. Anything else would need a DTD, which
/// XMIR does not carry, so it is refused instead of being guessed at.
fn resolve(name: &str) -> Result<&'static str, String> {
    match name {
        "amp" => Ok("&"),
        "lt" => Ok("<"),
        "gt" => Ok(">"),
        "quot" => Ok("\""),
        "apos" => Ok("'"),
        other => Err(format!("unknown entity &{other};")),
    }
}

/// Hand a finished element to its parent, or make it the root.
///
/// The text picked up around child elements is the indentation of the source
/// file, so it is dropped once the element turns out to have children. Text
/// that is not blank in that position is mixed content: one of the two would be
/// lost on the way out, so it is refused rather than silently flattened.
fn adopt(
    stack: &mut [Element],
    root: &mut Option<Element>,
    mut done: Element,
) -> Result<(), String> {
    if !done.children.is_empty() {
        match &done.text {
            Some(text) if !text.trim().is_empty() => {
                return Err(format!("<{}> holds both a datum and children", done.tag));
            }
            _ => done.text = None,
        }
    }
    match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None => *root = Some(done),
    }
    Ok(())
}

fn element(start: &quick_xml::events::BytesStart) -> Result<Element, String> {
    let mut attributes = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        attributes.push((
            String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
            String::from_utf8_lossy(attr.value.as_ref()).into_owned(),
        ));
    }
    Ok(Element {
        tag: String::from_utf8_lossy(start.name().as_ref()).into_owned(),
        attributes,
        text: None,
        children: Vec::new(),
    })
}

fn write(element: &Element, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    out.push('<');
    out.push_str(&element.tag);
    for (key, value) in &element.attributes {
        out.push_str(&format!(" {key}=\"{value}\""));
    }
    if let Some(text) = &element.text {
        out.push('>');
        out.push_str(&quick_xml::escape::escape(text));
        out.push_str(&format!("</{}>\n", element.tag));
        return;
    }
    if element.children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    for child in &element.children {
        write(child, depth + 1, out);
    }
    out.push_str(&pad);
    out.push_str(&format!("</{}>\n", element.tag));
}
