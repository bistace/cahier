use nu_ansi_term::Color;
use reedline::{Highlighter, StyledText};
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

pub struct SyntectHighlighter {
    theme_name: String,
}

impl SyntectHighlighter {
    pub fn new(theme_name: String) -> Self {
        // Initialize singletons if not already done
        let _ = SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines);
        let _ = THEME_SET.get_or_init(ThemeSet::load_defaults);

        Self { theme_name }
    }

    fn get_syntax_set() -> &'static SyntaxSet {
        SYNTAX_SET.get().expect("SyntaxSet not initialized")
    }

    fn get_theme_set() -> &'static ThemeSet {
        THEME_SET.get().expect("ThemeSet not initialized")
    }

    fn get_syntax_reference(&self) -> &syntect::parsing::SyntaxReference {
        let syntax_set = Self::get_syntax_set();
        let shell = std::env::var("SHELL").unwrap_or_default();

        if shell.ends_with("zsh") {
            syntax_set.find_syntax_by_name("Zsh").unwrap_or_else(|| {
                syntax_set
                    .find_syntax_by_name("Shell-Unix-Generic")
                    .unwrap()
            })
        } else if shell.ends_with("bash") {
            syntax_set.find_syntax_by_name("Bash").unwrap_or_else(|| {
                syntax_set
                    .find_syntax_by_name("Shell-Unix-Generic")
                    .unwrap()
            })
        } else {
            // Fallback
            syntax_set
                .find_syntax_by_name("Shell-Unix-Generic")
                .unwrap_or_else(|| syntax_set.find_syntax_plain_text())
        }
    }
}

impl Highlighter for SyntectHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled_text = StyledText::new();

        let theme_set = Self::get_theme_set();
        let theme = if theme_set.themes.contains_key(&self.theme_name) {
            &theme_set.themes[&self.theme_name]
        } else {
            // Fallback to default if configured theme not found
            // We only log once per instance/session to avoid spamming,
            // but here we can't easily track state. We'll just use fallback silently or with low-level debug.
            // eprintln!("Warning: Theme '{}' not found, using 'base16-ocean.dark'", self.theme_name);
            &theme_set.themes["base16-ocean.dark"]
        };

        let syntax_set = Self::get_syntax_set();
        let syntax = self.get_syntax_reference();
        let mut highlighter = HighlightLines::new(syntax, theme);

        for (style, text) in highlighter
            .highlight_line(line, syntax_set)
            .unwrap_or_default()
        {
            let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);

            // Note: Reedline's StyledText doesn't support background color easily per-span in the same way,
            // or rather, standard practice is usually just foreground for syntax highlighting in terminals
            // to avoid clashing with terminal background. We'll focus on foreground.

            let mut nu_style = nu_ansi_term::Style::new().fg(fg);

            if style
                .font_style
                .contains(syntect::highlighting::FontStyle::BOLD)
            {
                nu_style = nu_style.bold();
            }
            if style
                .font_style
                .contains(syntect::highlighting::FontStyle::ITALIC)
            {
                nu_style = nu_style.italic();
            }
            if style
                .font_style
                .contains(syntect::highlighting::FontStyle::UNDERLINE)
            {
                nu_style = nu_style.underline();
            }

            styled_text.push((nu_style, text.to_string()));
        }

        styled_text
    }
}
