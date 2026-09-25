//! Presentation-neutral documents.
//!
//! Analyzers and knowledge pages describe *what* to show as a [`Document`];
//! the UI decides *how* (colors, wrapping, scrolling). This keeps regex,
//! networking and knowledge code free of any terminal dependency, and lets
//! the same document be rendered in the TUI or printed as plain text.

/// Something a document can point to and the user can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A knowledge entry by id.
    Entry(String),
    /// A command line to explain; `note` is shown above the breakdown.
    Command { line: String, note: Option<String> },
    /// A category listing.
    Category(String),
}

/// Semantic emphasis; the UI maps it to colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Normal,
    Accent,
    Muted,
    Info,
    Success,
    Warning,
    Danger,
}

/// Aligned table row. The first cell is emphasized, the last one wraps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<String>,
    pub indent: u16,
    pub tone: Tone,
    pub link: Option<Link>,
}

impl Row {
    pub fn new<I, S>(cells: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            cells: cells.into_iter().map(Into::into).collect(),
            indent: 0,
            tone: Tone::Normal,
            link: None,
        }
    }

    pub fn indent(mut self, indent: u16) -> Self {
        self.indent = indent;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn link(mut self, link: Link) -> Self {
        self.link = Some(link);
        self
    }
}

/// Bullet/tree item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub text: String,
    pub detail: Option<String>,
    pub link: Option<Link>,
}

impl Item {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            detail: None,
            link: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn link(mut self, link: Link) -> Self {
        self.link = Some(link);
        self
    }
}

/// A numbered step with an optional command and its rationale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepItem {
    pub title: String,
    pub command: Option<String>,
    pub why: String,
    pub link: Option<Link>,
}

/// A node of a data-flow diagram (`grep` → stdout → pipe → `sort`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowNode {
    pub label: String,
    /// Edge drawn below this node, towards the next one.
    pub edge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Paragraph(String),
    /// A command or pattern with an optional caption, optionally openable.
    Code {
        text: String,
        caption: Option<String>,
        link: Option<Link>,
    },
    Table(Vec<Row>),
    List(Vec<Item>),
    Steps(Vec<StepItem>),
    Tree {
        root: String,
        children: Vec<Item>,
    },
    Flow(Vec<FlowNode>),
    Note {
        tone: Tone,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub title: String,
    pub subtitle: Option<String>,
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            blocks: Vec::new(),
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn push(&mut self, block: Block) -> &mut Self {
        self.blocks.push(block);
        self
    }

    pub fn heading(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(Block::Heading(text.into()))
    }

    pub fn paragraph(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(Block::Paragraph(text.into()))
    }

    pub fn code(&mut self, text: impl Into<String>, link: Option<Link>) -> &mut Self {
        self.push(Block::Code {
            text: text.into(),
            caption: None,
            link,
        })
    }

    /// A command followed by a short explanation.
    pub fn example(
        &mut self,
        text: impl Into<String>,
        caption: &str,
        link: Option<Link>,
    ) -> &mut Self {
        self.push(Block::Code {
            text: text.into(),
            caption: (!caption.is_empty()).then(|| caption.to_string()),
            link,
        })
    }

    pub fn note(&mut self, tone: Tone, text: impl Into<String>) -> &mut Self {
        self.push(Block::Note {
            tone,
            text: text.into(),
        })
    }

    pub fn table(&mut self, rows: Vec<Row>) -> &mut Self {
        if !rows.is_empty() {
            self.push(Block::Table(rows));
        }
        self
    }

    pub fn list(&mut self, items: Vec<Item>) -> &mut Self {
        if !items.is_empty() {
            self.push(Block::List(items));
        }
        self
    }

    /// Links in reading order — the order the detail view cycles through.
    pub fn links(&self) -> Vec<&Link> {
        let mut out = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Code { link: Some(l), .. } => out.push(l),
                Block::Steps(steps) => out.extend(steps.iter().filter_map(|s| s.link.as_ref())),
                Block::Table(rows) => out.extend(rows.iter().filter_map(|r| r.link.as_ref())),
                Block::List(items)
                | Block::Tree {
                    children: items, ..
                } => {
                    out.extend(items.iter().filter_map(|i| i.link.as_ref()));
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_in_reading_order() {
        let mut d = Document::new("t");
        d.code(
            "ls -la",
            Some(Link::Command {
                line: "ls -la".into(),
                note: None,
            }),
        )
        .table(vec![
            Row::new(["a", "b"]).link(Link::Entry("a".into())),
            Row::new(["c"]),
        ])
        .push(Block::Tree {
            root: "r".into(),
            children: vec![Item::new("x").link(Link::Entry("x".into()))],
        });
        let links: Vec<_> = d.links().into_iter().cloned().collect();
        assert_eq!(links.len(), 3);
        assert_eq!(links[2], Link::Entry("x".into()));
    }

    #[test]
    fn empty_tables_and_lists_are_skipped() {
        let mut d = Document::new("t");
        d.table(Vec::new()).list(Vec::new());
        assert!(d.blocks.is_empty());
    }
}
