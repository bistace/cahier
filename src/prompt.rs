use std::borrow::Cow;
use reedline::{Prompt, PromptEditMode, PromptHistorySearch};

#[derive(Clone)]
pub struct CahierPrompt;

impl CahierPrompt {
    pub fn new() -> Self {
        Self
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

        Cow::Owned(format!("{}@{}:{}\n", username, hostname, cwd))
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

