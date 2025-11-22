use std::borrow::Cow;
use reedline::{Prompt, PromptEditMode, PromptHistorySearch};
use crossterm::style::Stylize;

#[derive(Clone)]
pub struct CahierPrompt {
    last_success: bool,
}

impl CahierPrompt {
    pub fn new() -> Self {
        Self { last_success: true }
    }

    pub fn set_last_success(&mut self, success: bool) {
        self.last_success = success;
    }
}

impl Prompt for CahierPrompt {
    fn render_prompt_left(&self) -> Cow<str> {
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        let hostname = std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "localhost".to_string());
            
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string());

        let prompt_str = format!("{}@{}:{}\n", username, hostname, cwd);
        
        let colored_prompt = if self.last_success {
            prompt_str.green()
        } else {
            prompt_str.red()
        };

        Cow::Owned(colored_prompt.to_string())
    }

    fn render_prompt_right(&self) -> Cow<str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<str> {
        Cow::Borrowed("> ")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<str> {
        Cow::Borrowed(".. ")
    }

    fn render_prompt_history_search_indicator(&self, history_search: PromptHistorySearch) -> Cow<str> {
        let prefix = match history_search.status {
            reedline::PromptHistorySearchStatus::Passing => "",
            reedline::PromptHistorySearchStatus::Failing => "failing ",
        };
        
        Cow::Owned(format!("({}reverse-search: {}) ", prefix, history_search.term))
    }
}
