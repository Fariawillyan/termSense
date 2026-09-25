//! Keyboard input: key events → [`Action`]s, and a small UTF-8 aware line
//! editor for the query.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

/// Intent of a key press, independent of the current view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Esc: leave the detail view, or quit from the search view.
    Back,
    Insert(char),
    Paste(String),
    Backspace,
    Delete,
    DeleteWord,
    ClearLine,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    PageUp,
    PageDown,
    /// Tab: complete (search) or next link (details).
    Next,
    /// Shift+Tab: previous link (details).
    Previous,
    /// Enter: open.
    Open,
    None,
}

pub fn map_key(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('c' | 'd') if ctrl => Action::Quit,
        KeyCode::Char('u') if ctrl => Action::ClearLine,
        KeyCode::Char('w') if ctrl => Action::DeleteWord,
        KeyCode::Char('h') if ctrl => Action::Backspace,
        KeyCode::Char('a') if ctrl => Action::Home,
        KeyCode::Char('e') if ctrl => Action::End,
        KeyCode::Char('p') if ctrl => Action::Up,
        KeyCode::Char('n') if ctrl => Action::Down,
        KeyCode::Char(c) if !ctrl && !alt => Action::Insert(c),
        KeyCode::Backspace if alt || ctrl => Action::DeleteWord,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Delete => Action::Delete,
        KeyCode::Esc => Action::Back,
        KeyCode::Enter => Action::Open,
        KeyCode::Tab => Action::Next,
        KeyCode::BackTab => Action::Previous,
        KeyCode::Left => Action::Left,
        KeyCode::Right => Action::Right,
        KeyCode::Home => Action::Home,
        KeyCode::End => Action::End,
        KeyCode::Up => Action::Up,
        KeyCode::Down => Action::Down,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        _ => Action::None,
    }
}

/// Single-line editor. The cursor is a byte offset on a char boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    cursor: usize,
}

impl LineEditor {
    pub fn new(text: &str) -> Self {
        let mut e = Self::default();
        e.set(text);
        e
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Display columns before the cursor.
    pub fn cursor_column(&self) -> usize {
        self.text[..self.cursor].width()
    }

    /// Replaces the text and moves the cursor to the end.
    pub fn set(&mut self, text: &str) {
        self.text = sanitize(text);
        self.cursor = self.text.len();
    }

    pub fn insert(&mut self, c: char) {
        let c = if c.is_control() { ' ' } else { c };
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        let s = sanitize(s);
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }

    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
            self.text.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    /// Deletes the word before the cursor (Ctrl+W).
    pub fn delete_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(char::is_whitespace).map_or(0, |i| {
            i + trimmed[i..].chars().next().map_or(1, char::len_utf8)
        });
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }
}

/// Newlines and tabs from pasted text become spaces; trailing line breaks
/// are dropped (a real trailing space is kept).
fn sanitize(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    // Every control char became a one-byte space, so bytes == chars here.
    let trailing = s.chars().rev().take_while(|c| c.is_control()).count();
    flat[..flat.len() - trailing].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_utf8_text() {
        let mut e = LineEditor::new("conexão");
        e.backspace();
        assert_eq!(e.text(), "conexã");
        e.backspace();
        assert_eq!(e.text(), "conex");
        e.home();
        e.right();
        e.insert('ã');
        assert_eq!(e.text(), "cãonex");
        assert_eq!(e.cursor_column(), 2);
        e.delete();
        assert_eq!(e.text(), "cãnex");
    }

    #[test]
    fn delete_word_and_clear() {
        let mut e = LineEditor::new("grep -rin ERROR ");
        e.delete_word();
        assert_eq!(e.text(), "grep -rin ");
        e.delete_word();
        assert_eq!(e.text(), "grep ");
        e.clear();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn paste_flattens_newlines() {
        let mut e = LineEditor::default();
        e.insert_str("ls -la\n| grep x\n");
        assert_eq!(e.text(), "ls -la | grep x");
    }

    #[test]
    fn key_mapping() {
        let k = |code, m| map_key(KeyEvent::new(code, m));
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), Action::Quit);
        assert_eq!(
            k(KeyCode::Char('C'), KeyModifiers::SHIFT),
            Action::Insert('C')
        );
        assert_eq!(k(KeyCode::Tab, KeyModifiers::NONE), Action::Next);
        assert_eq!(k(KeyCode::Esc, KeyModifiers::NONE), Action::Back);
        assert_eq!(
            k(KeyCode::Char('w'), KeyModifiers::CONTROL),
            Action::DeleteWord
        );
    }
}
