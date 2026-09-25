//! Application state: the query, the current answer, the selection and the
//! stack of open detail pages. Pure state transitions — no terminal I/O, so
//! the whole interaction is unit-testable.

use crate::assistant::{Assistant, Mode, Response, Suggestion};
use crate::document::Document;
use crate::input::{Action, LineEditor};

/// A document opened full-screen (Enter).
#[derive(Debug, Clone)]
pub struct DetailView {
    pub doc: Document,
    pub scroll: u16,
    /// Index into `doc.links()` of the focused link.
    pub focus: Option<usize>,
    /// Scroll the focused link into view on the next render.
    pub follow_focus: bool,
}

impl DetailView {
    fn new(doc: Document) -> Self {
        Self {
            doc,
            scroll: 0,
            focus: None,
            follow_focus: false,
        }
    }
}

pub struct App {
    assistant: Assistant,
    pub editor: LineEditor,
    pub response: Response,
    pub selected: usize,
    /// First visible row of the result list (kept by the UI).
    pub list_offset: usize,
    pub preview: Document,
    pub preview_scroll: u16,
    /// Detail pages; the last one is visible. Empty = search view.
    pub details: Vec<DetailView>,
    /// Height of the main area at the last render, for paging.
    pub viewport: u16,
    pub quit: bool,
}

impl App {
    pub fn new(mut assistant: Assistant, query: &str) -> Self {
        let editor = LineEditor::new(query);
        let response = assistant.respond(editor.text());
        let mut app = Self {
            assistant,
            editor,
            response,
            selected: 0,
            list_offset: 0,
            preview: Document::default(),
            preview_scroll: 0,
            details: Vec::new(),
            viewport: 20,
            quit: false,
        };
        app.update_preview();
        app
    }

    pub fn mode(&self) -> Mode {
        self.response.mode
    }

    pub fn suggestions(&self) -> &[Suggestion] {
        &self.response.suggestions
    }

    pub fn selected_suggestion(&self) -> Option<&Suggestion> {
        self.response.suggestions.get(self.selected)
    }

    pub fn detail_mut(&mut self) -> Option<&mut DetailView> {
        self.details.last_mut()
    }

    pub fn knowledge_size(&self) -> usize {
        self.assistant.repository().len()
    }

    pub fn warnings(&self) -> &[String] {
        self.assistant.repository().warnings()
    }

    pub fn handle(&mut self, action: Action) {
        if action == Action::Quit {
            self.quit = true;
            return;
        }
        if self.details.is_empty() {
            self.handle_search(action);
        } else {
            self.handle_detail(action);
        }
    }

    fn handle_search(&mut self, action: Action) {
        let before = self.editor.text().to_string();
        match action {
            Action::Back => self.quit = true,
            Action::Insert(c) => self.editor.insert(c),
            Action::Paste(s) => self.editor.insert_str(&s),
            Action::Backspace => self.editor.backspace(),
            Action::Delete => self.editor.delete(),
            Action::DeleteWord => self.editor.delete_word(),
            Action::ClearLine => self.editor.clear(),
            Action::Left => self.editor.left(),
            Action::Right => self.editor.right(),
            Action::Home => self.editor.home(),
            Action::End => self.editor.end(),
            Action::Up => self.select(self.selected.saturating_sub(1)),
            Action::Down => self.select(self.selected + 1),
            Action::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(self.page()),
            Action::PageDown => {
                self.preview_scroll = self.preview_scroll.saturating_add(self.page())
            }
            Action::Next => {
                if let Some(completion) = self
                    .selected_suggestion()
                    .and_then(|s| s.completion.clone())
                {
                    self.editor.set(&completion);
                }
            }
            Action::Open => {
                if self.selected_suggestion().is_some() {
                    self.details.push(DetailView::new(self.preview.clone()));
                }
            }
            Action::Previous | Action::Quit | Action::None => {}
        }
        if self.editor.text() != before {
            self.refresh();
        }
    }

    fn handle_detail(&mut self, action: Action) {
        let page = self.page();
        let Some(view) = self.details.last_mut() else {
            return;
        };
        let links = view.doc.links().len();
        match action {
            Action::Back | Action::Backspace | Action::Left | Action::Insert('q') => {
                self.details.pop();
            }
            Action::Up | Action::Insert('k') => view.scroll = view.scroll.saturating_sub(1),
            Action::Down | Action::Insert('j') => view.scroll = view.scroll.saturating_add(1),
            Action::PageUp => view.scroll = view.scroll.saturating_sub(page),
            Action::PageDown | Action::Insert(' ') => {
                view.scroll = view.scroll.saturating_add(page)
            }
            Action::Home | Action::Insert('g') => view.scroll = 0,
            Action::End | Action::Insert('G') => view.scroll = u16::MAX,
            Action::Next | Action::Right if links > 0 => {
                view.focus = Some(view.focus.map_or(0, |f| (f + 1) % links));
                view.follow_focus = true;
            }
            Action::Previous if links > 0 => {
                view.focus = Some(view.focus.map_or(links - 1, |f| (f + links - 1) % links));
                view.follow_focus = true;
            }
            Action::Open => {
                let link = view
                    .focus
                    .and_then(|f| view.doc.links().get(f).map(|l| (*l).clone()));
                if let Some(link) = link {
                    let doc = self.assistant.open(&link);
                    self.details.push(DetailView::new(doc));
                }
            }
            _ => {}
        }
    }

    fn page(&self) -> u16 {
        self.viewport.saturating_sub(2).max(1)
    }

    fn select(&mut self, index: usize) {
        let last = self.response.suggestions.len().saturating_sub(1);
        let index = index.min(last);
        if index != self.selected {
            self.selected = index;
            self.update_preview();
        }
    }

    /// Recomputes the answer after the query changed.
    fn refresh(&mut self) {
        self.response = self.assistant.respond(self.editor.text());
        self.selected = 0;
        self.list_offset = 0;
        self.update_preview();
    }

    fn update_preview(&mut self) {
        self.preview = match self.selected_suggestion() {
            Some(s) => self.assistant.preview(s),
            None => {
                let mut doc = Document::new("Nenhum resultado");
                doc.paragraph(format!(
                    "Nada encontrado para \"{}\". Tente outras palavras, um comando (grep, ss, git) ou apague a busca para ver os temas.",
                    self.editor.text().trim()
                ));
                doc
            }
        };
        self.preview_scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::SuggestionKind;
    use crate::config::Config;
    use crate::knowledge::Repository;

    fn app(query: &str) -> App {
        let assistant = Assistant::new(Repository::embedded().unwrap(), &Config::default());
        App::new(assistant, query)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Action::Insert(c));
        }
    }

    #[test]
    fn results_update_on_every_keystroke() {
        let mut a = app("");
        assert_eq!(a.mode(), Mode::Home);
        type_text(&mut a, "gr");
        assert_eq!(a.mode(), Mode::Search);
        assert_eq!(a.suggestions()[0].title, "grep");
        assert_eq!(a.preview.title, "grep");
        a.handle(Action::Backspace);
        a.handle(Action::Backspace);
        assert_eq!(a.mode(), Mode::Home);
    }

    #[test]
    fn tab_completes_and_enter_opens() {
        let mut a = app("");
        type_text(&mut a, "gre");
        a.handle(Action::Next);
        assert_eq!(a.editor.text(), "grep ");
        type_text(&mut a, "-");
        assert_eq!(a.suggestions()[0].kind, SuggestionKind::Option);

        a.handle(Action::Open);
        assert_eq!(a.details.len(), 1);
        a.handle(Action::Back);
        assert!(a.details.is_empty());
        assert!(!a.quit);
        a.handle(Action::Back);
        assert!(a.quit);
    }

    #[test]
    fn navigating_links_in_details() {
        let mut a = app("grep");
        a.handle(Action::Open);
        assert_eq!(a.details.last().unwrap().doc.title, "grep");
        a.handle(Action::Next);
        assert_eq!(a.details.last().unwrap().focus, Some(0));
        a.handle(Action::Open);
        assert_eq!(a.details.len(), 2, "o link focado abre outra página");
        a.handle(Action::Back);
        assert_eq!(a.details.len(), 1);
    }

    #[test]
    fn selection_is_clamped() {
        let mut a = app("gr");
        for _ in 0..500 {
            a.handle(Action::Down);
        }
        assert_eq!(a.selected, a.suggestions().len() - 1);
        a.handle(Action::Up);
        assert_eq!(a.selected, a.suggestions().len() - 2);
    }

    #[test]
    fn paste_and_ctrl_c() {
        let mut a = app("");
        a.handle(Action::Paste("regex ^[0-9]+$".into()));
        assert_eq!(a.mode(), Mode::Regex);
        a.handle(Action::Quit);
        assert!(a.quit);
    }
}
