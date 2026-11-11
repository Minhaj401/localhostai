//! Keyboard state for online model discovery, separate from the chat draft.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use privateml_core::model_hub::{HubResults, ModelSource};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct ModelBrowser {
    pub query: String,
    pub cursor: usize,
    pub results: HubResults,
    pub selected: Option<usize>,
    pub editing: bool,
    pub pending: bool,
    pub error: Option<String>,
    pub request_id: u64,
}

pub enum BrowserAction {
    None,
    Search { request_id: u64, query: String },
    Download { source: ModelSource, name: String },
}

impl ModelBrowser {
    pub fn rows(&self) -> usize {
        self.results.repositories.len() + self.results.models.len()
    }

    fn edited(&mut self) {
        self.request_id = self.request_id.wrapping_add(1);
        self.pending = false;
        self.error = None;
        self.results = HubResults::default();
        self.selected = None;
    }

    pub fn paste(&mut self, value: &str) {
        if !self.editing {
            return;
        }
        let value: String = value.chars().filter(|c| !c.is_control()).collect();
        if self.query.len().saturating_add(value.len()) > 4096 {
            self.error = Some("Search or URL is too long".into());
            return;
        }
        self.query.insert_str(self.cursor, &value);
        self.cursor += value.len();
        self.edited();
    }

    fn search(&mut self, query: String) -> BrowserAction {
        self.edited();
        self.query = query.trim().to_string();
        self.cursor = self.query.len();
        self.pending = true;
        BrowserAction::Search {
            request_id: self.request_id,
            query: self.query.clone(),
        }
    }

    pub fn apply(&mut self, request_id: u64, result: anyhow::Result<HubResults>) {
        if request_id != self.request_id {
            return;
        }
        self.pending = false;
        match result {
            Ok(results) => {
                self.results = results;
                self.selected = (self.rows() > 0).then_some(0);
                self.editing = self.selected.is_none();
            }
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                self.editing = true;
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent, can_download: bool) -> BrowserAction {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Tab {
            self.editing = !self.editing || self.rows() == 0;
            return BrowserAction::None;
        }
        if !self.editing {
            match key.code {
                KeyCode::Up => self.selected = self.selected.map(|i| i.saturating_sub(1)),
                KeyCode::Down => {
                    self.selected = self
                        .selected
                        .map(|i| (i + 1).min(self.rows().saturating_sub(1)))
                }
                KeyCode::Enter => {
                    if let Some(index) = self.selected {
                        if let Some(repo) = self.results.repositories.get(index) {
                            return self.search(repo.id.clone());
                        }
                        if let Some(model) = self
                            .results
                            .models
                            .get(index.saturating_sub(self.results.repositories.len()))
                        {
                            if can_download {
                                return BrowserAction::Download {
                                    source: model.source.clone(),
                                    name: model.display_name.clone(),
                                };
                            }
                            self.error =
                                Some("Wait for the current model operation to finish".into());
                        }
                    }
                }
                _ => {}
            }
            return BrowserAction::None;
        }
        match key.code {
            KeyCode::Enter => return self.search(self.query.clone()),
            KeyCode::Down if self.rows() > 0 => {
                self.editing = false;
                self.selected = Some(0);
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.query.len(),
            KeyCode::Left => {
                self.cursor = self.query[..self.cursor]
                    .grapheme_indices(true)
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            KeyCode::Right => {
                if let Some(grapheme) = self.query[self.cursor..].graphemes(true).next() {
                    self.cursor += grapheme.len();
                }
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let start = self.query[..self.cursor]
                    .grapheme_indices(true)
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.query.drain(start..self.cursor);
                self.cursor = start;
                self.edited();
            }
            KeyCode::Delete => {
                if let Some(grapheme) = self.query[self.cursor..].graphemes(true).next() {
                    self.query.drain(self.cursor..self.cursor + grapheme.len());
                    self.edited();
                }
            }
            KeyCode::Char('u') if control => {
                self.query.clear();
                self.cursor = 0;
                self.edited();
            }
            KeyCode::Char(c) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.paste(&c.to_string())
            }
            _ => {}
        }
        BrowserAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_links_are_searches_and_late_results_cannot_replace_new_input() {
        let mut browser = ModelBrowser {
            editing: true,
            ..Default::default()
        };
        browser.paste("https://example.com/model.gguf\n");
        let BrowserAction::Search { request_id, query } =
            browser.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true)
        else {
            panic!("search expected")
        };
        assert_eq!(query, "https://example.com/model.gguf");
        browser.key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            true,
        );
        browser.paste("新しいモデル");
        browser.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), true);
        browser.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), true);
        assert_eq!(browser.query, "新しいモル");
        browser.apply(request_id, Err(anyhow::anyhow!("old error")));
        assert!(browser.error.is_none());
        assert!(!browser.pending);
    }
}
