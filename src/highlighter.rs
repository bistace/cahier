use reedline::{Highlighter, StyledText};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use nu_ansi_term::Color;

pub struct SyntectHighlighter {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
    theme_name: String,
}

impl SyntectHighlighter {
    pub fn new(theme_name: String) -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
            theme_name,
        }
    }

    fn get_syntax_reference(&self) -> &syntect::parsing::SyntaxReference {
        let shell = std::env::var("SHELL").unwrap_or_default();
        
        if shell.ends_with("zsh") {
             self.syntax_set.find_syntax_by_name("Zsh").unwrap_or_else(|| self.syntax_set.find_syntax_by_name("Shell-Unix-Generic").unwrap())
        } else if shell.ends_with("bash") {
             self.syntax_set.find_syntax_by_name("Bash").unwrap_or_else(|| self.syntax_set.find_syntax_by_name("Shell-Unix-Generic").unwrap())
        } else {
            // Fallback
            self.syntax_set.find_syntax_by_name("Shell-Unix-Generic")
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
        }
    }
}

impl Highlighter for SyntectHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled_text = StyledText::new();
        
        let theme = if self.theme_set.themes.contains_key(&self.theme_name) {
            &self.theme_set.themes[&self.theme_name]
        } else {
            // Fallback to default if configured theme not found
            eprintln!("Warning: Theme '{}' not found, using 'base16-ocean.dark'", self.theme_name);
            &self.theme_set.themes["base16-ocean.dark"]
        };

        let syntax = self.get_syntax_reference();
        let mut highlighter = HighlightLines::new(syntax, theme);

        for (style, text) in highlighter.highlight_line(line, &self.syntax_set).unwrap_or_default() {
            let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
            
            // Note: Reedline's StyledText doesn't support background color easily per-span in the same way, 
            // or rather, standard practice is usually just foreground for syntax highlighting in terminals 
            // to avoid clashing with terminal background. We'll focus on foreground.
            
            let mut nu_style = nu_ansi_term::Style::new().fg(fg);
            
            if style.font_style.contains(syntect::highlighting::FontStyle::BOLD) {
                nu_style = nu_style.bold();
            }
            if style.font_style.contains(syntect::highlighting::FontStyle::ITALIC) {
                nu_style = nu_style.italic();
            }
            if style.font_style.contains(syntect::highlighting::FontStyle::UNDERLINE) {
                nu_style = nu_style.underline();
            }

            styled_text.push((nu_style, text.to_string()));
        }

        styled_text
    }
}
