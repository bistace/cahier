use anyhow::Result;
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io::{self, Stdout};
use tui_textarea::TextArea;

use crate::common;
use crate::db::{Database, Direction as DbDirection, EntrySummary, Snippet, SnippetScope};

pub fn run(db: Database) -> Result<Option<String>> {
    let global_db = Database::init(common::global_snippets_db_path())?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_result = run_app(&mut terminal, db, global_db);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    app_result
}

struct App<'a> {
    db: Database,
    global_db: Database,
    entries: Vec<EntrySummary>,
    entry_list_state: ListState,
    snippets: Vec<Snippet>,
    snippet_list_state: ListState,
    screen: Screen,
    should_quit: bool,
    show_output_fullscreen: bool,
    input_mode: InputMode,
    annotation_input: TextArea<'a>,
    snippet_form: Option<SnippetForm>,
    fullscreen_scroll: u16,
    selected_command: Option<String>,
    current_output_cache: Option<String>,
    preview_collapsed: bool,
    status_message: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    History,
    Snippets,
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    EditingAnnotation,
    ConfirmDeleteEntry,
    CreatingSnippet,
    ConfirmDeleteSnippet,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SnippetField {
    Name,
    Description,
    Scope,
    Tags,
}

struct SnippetForm {
    command: String,
    name: String,
    description: String,
    tags: String,
    scope: SnippetScope,
    active_field: SnippetField,
}

impl SnippetForm {
    fn new(command: String) -> Self {
        Self {
            command,
            name: String::new(),
            description: String::new(),
            tags: String::new(),
            scope: SnippetScope::Project,
            active_field: SnippetField::Name,
        }
    }

    fn next_field(&mut self) {
        self.active_field = match self.active_field {
            SnippetField::Name => SnippetField::Description,
            SnippetField::Description => SnippetField::Scope,
            SnippetField::Scope => SnippetField::Tags,
            SnippetField::Tags => SnippetField::Name,
        };
    }

    fn previous_field(&mut self) {
        self.active_field = match self.active_field {
            SnippetField::Name => SnippetField::Tags,
            SnippetField::Description => SnippetField::Name,
            SnippetField::Scope => SnippetField::Description,
            SnippetField::Tags => SnippetField::Scope,
        };
    }

    fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            SnippetScope::Project => SnippetScope::Global,
            SnippetScope::Global => SnippetScope::Project,
        };
    }

    fn active_text_mut(&mut self) -> Option<&mut String> {
        match self.active_field {
            SnippetField::Name => Some(&mut self.name),
            SnippetField::Description => Some(&mut self.description),
            SnippetField::Tags => Some(&mut self.tags),
            SnippetField::Scope => None,
        }
    }
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    db: Database,
    global_db: Database,
) -> Result<Option<String>> {
    let mut app = App::new(db, global_db)?;

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match app.input_mode {
                    InputMode::Normal => app.handle_normal_mode(key, terminal)?,
                    InputMode::EditingAnnotation => app.handle_annotation_mode(key)?,
                    InputMode::ConfirmDeleteEntry => app.handle_delete_entry_mode(key)?,
                    InputMode::CreatingSnippet => app.handle_create_snippet_mode(key)?,
                    InputMode::ConfirmDeleteSnippet => app.handle_delete_snippet_mode(key)?,
                }
            }
        }

        if app.should_quit {
            return Ok(app.selected_command);
        }
    }
}

impl<'a> App<'a> {
    fn new(db: Database, global_db: Database) -> Result<Self> {
        let entries = db.get_all_entry_summaries()?;
        let mut entry_list_state = ListState::default();
        if !entries.is_empty() {
            entry_list_state.select(Some(entries.len() - 1));
        }

        let mut annotation_input = TextArea::default();
        annotation_input.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Edit Annotation"),
        );
        annotation_input.set_style(Style::default().fg(Color::Yellow));
        annotation_input.set_cursor_line_style(Style::default());

        let mut app = Self {
            db,
            global_db,
            entries,
            entry_list_state,
            snippets: Vec::new(),
            snippet_list_state: ListState::default(),
            screen: Screen::History,
            should_quit: false,
            show_output_fullscreen: false,
            input_mode: InputMode::Normal,
            annotation_input,
            snippet_form: None,
            fullscreen_scroll: 0,
            selected_command: None,
            current_output_cache: None,
            preview_collapsed: false,
            status_message: None,
        };
        app.update_output_cache();
        app.refresh_snippets()?;
        Ok(app)
    }

    fn handle_normal_mode(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        if self.show_output_fullscreen {
            return self.handle_fullscreen_mode(key, terminal);
        }

        match self.screen {
            Screen::History => self.handle_history_mode(key, terminal),
            Screen::Snippets => self.handle_snippet_mode(key, terminal),
        }
    }

    fn handle_fullscreen_mode(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                self.scroll_fullscreen_up();
                terminal.clear()?;
            }
            KeyCode::Down => {
                self.scroll_fullscreen_down();
                terminal.clear()?;
            }
            KeyCode::Enter | KeyCode::Char('q') => {
                self.toggle_output_fullscreen();
                terminal.clear()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_history_mode(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                self.next_entry();
                terminal.clear()?;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.previous_entry();
                terminal.clear()?;
            }
            KeyCode::Char('J') => self.move_entry(DbDirection::Down)?,
            KeyCode::Char('K') => self.move_entry(DbDirection::Up)?,
            KeyCode::Char('d') => {
                if self.entry_list_state.selected().is_some() {
                    self.input_mode = InputMode::ConfirmDeleteEntry;
                }
            }
            KeyCode::Char('a') => self.start_editing_annotation(),
            KeyCode::Char('b') => self.start_creating_snippet(),
            KeyCode::Char('S') => {
                self.screen = Screen::Snippets;
                self.refresh_snippets()?;
                terminal.clear()?;
            }
            KeyCode::Char(' ') => self.insert_separator()?,
            KeyCode::Char('s') => {
                if let Some(entry) = self.selected_entry() {
                    self.selected_command = Some(entry.command.clone());
                    self.should_quit = true;
                }
            }
            KeyCode::Enter => {
                self.toggle_output_fullscreen();
                terminal.clear()?;
            }
            KeyCode::Char('p') => {
                self.toggle_preview_collapsed();
                terminal.clear()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_snippet_mode(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Char('q') => {
                self.screen = Screen::History;
                terminal.clear()?;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.next_snippet();
                terminal.clear()?;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.previous_snippet();
                terminal.clear()?;
            }
            KeyCode::Char('d') => {
                if self.snippet_list_state.selected().is_some() {
                    self.input_mode = InputMode::ConfirmDeleteSnippet;
                }
            }
            KeyCode::Char('s') => {
                if let Some(snippet) = self.selected_snippet() {
                    self.selected_command = Some(snippet.command.clone());
                    self.should_quit = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_annotation_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.save_annotation()?,
            _ => {
                self.annotation_input.input(key);
            }
        }
        Ok(())
    }

    fn handle_delete_entry_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.delete_entry()?;
                self.input_mode = InputMode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_create_snippet_mode(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.snippet_form.as_mut() else {
            self.input_mode = InputMode::Normal;
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => {
                self.snippet_form = None;
                self.input_mode = InputMode::Normal;
                self.status_message = Some("Snippet creation cancelled".to_string());
            }
            KeyCode::Enter => {
                self.save_snippet()?;
            }
            KeyCode::Tab | KeyCode::Down => form.next_field(),
            KeyCode::BackTab | KeyCode::Up => form.previous_field(),
            KeyCode::Left | KeyCode::Right if form.active_field == SnippetField::Scope => {
                form.toggle_scope();
            }
            KeyCode::Char(' ') if form.active_field == SnippetField::Scope => {
                form.toggle_scope();
            }
            _ => {
                if let Some(value) = form.active_text_mut() {
                    handle_text_input(value, key);
                }
            }
        }
        Ok(())
    }

    fn handle_delete_snippet_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.delete_snippet()?;
                self.input_mode = InputMode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    fn selected_entry(&self) -> Option<&EntrySummary> {
        self.entry_list_state
            .selected()
            .and_then(|index| self.entries.get(index))
    }

    fn selected_snippet(&self) -> Option<&Snippet> {
        self.snippet_list_state
            .selected()
            .and_then(|index| self.snippets.get(index))
    }

    fn update_output_cache(&mut self) {
        if let Some(entry) = self.selected_entry() {
            self.current_output_cache = self
                .db
                .get_entry_output(entry.id)
                .ok()
                .map(|s| process_carriage_returns(&s));
        } else {
            self.current_output_cache = None;
        }
    }

    fn next_entry(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let index = match self.entry_list_state.selected() {
            Some(i) if i < self.entries.len() - 1 => i + 1,
            Some(_) => 0,
            None => 0,
        };
        self.entry_list_state.select(Some(index));
        self.update_output_cache();
    }

    fn previous_entry(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let index = match self.entry_list_state.selected() {
            Some(0) => self.entries.len() - 1,
            Some(i) => i - 1,
            None => 0,
        };
        self.entry_list_state.select(Some(index));
        self.update_output_cache();
    }

    fn next_snippet(&mut self) {
        if self.snippets.is_empty() {
            return;
        }
        let index = match self.snippet_list_state.selected() {
            Some(i) if i < self.snippets.len() - 1 => i + 1,
            Some(_) => 0,
            None => 0,
        };
        self.snippet_list_state.select(Some(index));
    }

    fn previous_snippet(&mut self) {
        if self.snippets.is_empty() {
            return;
        }
        let index = match self.snippet_list_state.selected() {
            Some(0) => self.snippets.len() - 1,
            Some(i) => i - 1,
            None => 0,
        };
        self.snippet_list_state.select(Some(index));
    }

    fn move_entry(&mut self, direction: DbDirection) -> Result<()> {
        if let Some(i) = self.entry_list_state.selected() {
            let id = self.entries[i].id;
            self.db.move_entry(id, direction)?;
            self.refresh_entries()?;

            let new_index = match direction {
                DbDirection::Up if i > 0 => i - 1,
                DbDirection::Down if i < self.entries.len().saturating_sub(1) => i + 1,
                _ => i,
            };
            if !self.entries.is_empty() {
                self.entry_list_state.select(Some(new_index));
            }
            self.update_output_cache();
        }
        Ok(())
    }

    fn delete_entry(&mut self) -> Result<()> {
        if let Some(i) = self.entry_list_state.selected() {
            let id = self.entries[i].id;
            self.db.delete_entry(id)?;
            self.refresh_entries()?;
            if self.entries.is_empty() {
                self.entry_list_state.select(None);
            } else {
                let index = i.min(self.entries.len() - 1);
                self.entry_list_state.select(Some(index));
            }
            self.update_output_cache();
        }
        Ok(())
    }

    fn insert_separator(&mut self) -> Result<()> {
        let current_rank = self
            .selected_entry()
            .map(|entry| entry.rank)
            .unwrap_or_default();

        self.db.insert_separator(current_rank + 1)?;
        self.refresh_entries()?;

        if let Some(i) = self.entry_list_state.selected() {
            if i < self.entries.len().saturating_sub(1) {
                self.entry_list_state.select(Some(i + 1));
            }
        } else if !self.entries.is_empty() {
            self.entry_list_state.select(Some(0));
        }

        self.update_output_cache();
        Ok(())
    }

    fn start_editing_annotation(&mut self) {
        if let Some(content) = self
            .selected_entry()
            .map(|entry| entry.annotation.clone().unwrap_or_default())
        {
            self.input_mode = InputMode::EditingAnnotation;
            self.annotation_input = TextArea::new(content.lines().map(|s| s.to_string()).collect());
            self.annotation_input.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Edit Annotation"),
            );
            self.annotation_input
                .set_style(Style::default().fg(Color::Yellow));
            self.annotation_input
                .set_cursor_line_style(Style::default());
            self.annotation_input
                .move_cursor(tui_textarea::CursorMove::Bottom);
            self.annotation_input
                .move_cursor(tui_textarea::CursorMove::End);
        }
    }

    fn save_annotation(&mut self) -> Result<()> {
        if let Some(i) = self.entry_list_state.selected() {
            let id = self.entries[i].id;
            let content = self.annotation_input.lines().join("\n");
            self.db.update_annotation(id, content)?;
            self.refresh_entries()?;
            self.input_mode = InputMode::Normal;

            self.annotation_input = TextArea::default();
            self.annotation_input.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Edit Annotation"),
            );
            self.annotation_input
                .set_style(Style::default().fg(Color::Yellow));
            self.annotation_input
                .set_cursor_line_style(Style::default());

            if !self.entries.is_empty() {
                self.entry_list_state
                    .select(Some(i.min(self.entries.len() - 1)));
            }
        }
        Ok(())
    }

    fn start_creating_snippet(&mut self) {
        let Some(entry) = self.selected_entry() else {
            self.status_message = Some("No command selected".to_string());
            return;
        };

        if entry.is_separator || entry.command.trim().is_empty() {
            self.status_message = Some("Separators cannot be saved as snippets".to_string());
            return;
        }

        self.snippet_form = Some(SnippetForm::new(entry.command.clone()));
        self.input_mode = InputMode::CreatingSnippet;
    }

    fn save_snippet(&mut self) -> Result<()> {
        let Some(form) = self.snippet_form.take() else {
            self.input_mode = InputMode::Normal;
            return Ok(());
        };

        if form.name.trim().is_empty() {
            self.snippet_form = Some(form);
            self.status_message = Some("Snippet name is required".to_string());
            return Ok(());
        }

        let description = trimmed_option(&form.description);
        let tags = trimmed_option(&form.tags);
        match form.scope {
            SnippetScope::Project => self.db.create_snippet(
                form.name.trim(),
                &form.command,
                description.as_deref(),
                SnippetScope::Project,
                tags.as_deref(),
            )?,
            SnippetScope::Global => self.global_db.create_snippet(
                form.name.trim(),
                &form.command,
                description.as_deref(),
                SnippetScope::Global,
                tags.as_deref(),
            )?,
        }

        self.refresh_snippets()?;
        self.input_mode = InputMode::Normal;
        self.status_message = Some(format!("Saved {} snippet", form.scope));
        Ok(())
    }

    fn delete_snippet(&mut self) -> Result<()> {
        let Some(snippet) = self.selected_snippet().cloned() else {
            return Ok(());
        };

        match snippet.scope {
            SnippetScope::Project => self.db.delete_snippet(snippet.id)?,
            SnippetScope::Global => self.global_db.delete_snippet(snippet.id)?,
        }

        self.refresh_snippets()?;
        if self.snippets.is_empty() {
            self.snippet_list_state.select(None);
        } else if let Some(i) = self.snippet_list_state.selected() {
            let index = i.min(self.snippets.len() - 1);
            self.snippet_list_state.select(Some(index));
        }
        self.status_message = Some("Snippet deleted".to_string());
        Ok(())
    }

    fn toggle_output_fullscreen(&mut self) {
        self.show_output_fullscreen = !self.show_output_fullscreen;
        self.fullscreen_scroll = 0;
    }

    fn toggle_preview_collapsed(&mut self) {
        self.preview_collapsed = !self.preview_collapsed;
    }

    fn scroll_fullscreen_up(&mut self) {
        if self.fullscreen_scroll > 0 {
            self.fullscreen_scroll -= 1;
        }
    }

    fn scroll_fullscreen_down(&mut self) {
        self.fullscreen_scroll += 1;
    }

    fn refresh_entries(&mut self) -> Result<()> {
        self.entries = self.db.get_all_entry_summaries()?;
        Ok(())
    }

    fn refresh_snippets(&mut self) -> Result<()> {
        self.snippets = self.db.get_all_snippets()?;
        self.snippets.extend(self.global_db.get_all_snippets()?);
        self.snippets.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.name.cmp(&right.name))
        });

        if self.snippets.is_empty() {
            self.snippet_list_state.select(None);
        } else if self.snippet_list_state.selected().is_none() {
            self.snippet_list_state.select(Some(0));
        } else if let Some(i) = self.snippet_list_state.selected() {
            self.snippet_list_state
                .select(Some(i.min(self.snippets.len() - 1)));
        }

        Ok(())
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    if app.show_output_fullscreen {
        render_fullscreen_output(f, app);
    } else {
        match app.screen {
            Screen::History => render_history_layout(f, app),
            Screen::Snippets => render_snippet_layout(f, app),
        }
    }

    match app.input_mode {
        InputMode::EditingAnnotation => render_annotation_popup(f, app),
        InputMode::ConfirmDeleteEntry => render_confirmation_popup(
            f,
            "Confirm Deletion",
            "Are you sure you want to delete this entry?\n\n(y)es / (n)o",
            Color::Red,
        ),
        InputMode::CreatingSnippet => render_create_snippet_popup(f, app),
        InputMode::ConfirmDeleteSnippet => render_confirmation_popup(
            f,
            "Delete Snippet",
            "Delete the selected snippet?\n\n(y)es / (n)o",
            Color::Red,
        ),
        InputMode::Normal => {}
    }
}

fn render_history_layout(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let content_constraints = if app.preview_collapsed {
        vec![Constraint::Percentage(100), Constraint::Min(0)]
    } else {
        vec![Constraint::Percentage(50), Constraint::Percentage(50)]
    };

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(content_constraints)
        .split(chunks[0]);

    let list_width = content_chunks[0].width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|entry| render_history_item(entry, list_width, app.preview_collapsed))
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("History"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, content_chunks[0], &mut app.entry_list_state);

    if !app.preview_collapsed {
        let preview = if app.entry_list_state.selected().is_some() {
            Paragraph::new(app.current_output_cache.as_deref().unwrap_or("Loading..."))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Output Preview"),
                )
                .wrap(Wrap { trim: true })
        } else {
            Paragraph::new("No command selected").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Output Preview"),
            )
        };
        f.render_widget(preview, content_chunks[1]);
    }

    let status_text = format_status_line(
        "j/k: Navigate | J/K: Move | d: Delete | a: Annotate | b: Save Snippet | S: Browse Snippets | Space: Separator | s: Send to REPL | Enter: Fullscreen | p: Toggle Preview | q: Quit",
        app.status_message.as_deref(),
    );
    let status =
        Paragraph::new(status_text).style(Style::default().bg(Color::Blue).fg(Color::White));
    f.render_widget(status, chunks[1]);
}

fn render_history_item(
    entry: &EntrySummary,
    list_width: usize,
    preview_collapsed: bool,
) -> ListItem<'static> {
    if entry.is_separator {
        let separator = " --- ";
        let content = Line::styled(separator, Style::default().fg(Color::DarkGray));
        return ListItem::new(content);
    }

    let annotation = entry.annotation.as_deref().unwrap_or("");
    let rank_color = match entry.exit_code {
        Some(0) => Color::Blue,
        _ => Color::Red,
    };

    if preview_collapsed {
        let rank_str = format!("[{}]", entry.rank);
        let rank_span = Span::styled(rank_str.clone(), Style::default().fg(rank_color));
        let mut lines = Vec::new();

        if !annotation.is_empty() {
            for line in textwrap::wrap(annotation, list_width) {
                lines.push(Line::styled(
                    line.to_string(),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }

        let indent = " ".repeat(rank_str.len() + 1);
        let full_command = format!("{} {}", rank_str, entry.command);
        let options = textwrap::Options::new(list_width).subsequent_indent(&indent);

        for (index, line) in textwrap::wrap(&full_command, &options).iter().enumerate() {
            if index == 0 {
                let line_str = line.to_string();
                if line_str.starts_with(&rank_str) {
                    let content_part = &line_str[rank_str.len()..];
                    lines.push(Line::from(vec![
                        rank_span.clone(),
                        Span::raw(content_part.to_string()),
                    ]));
                } else {
                    lines.push(Line::raw(line_str));
                }
            } else {
                lines.push(Line::raw(line.to_string()));
            }
        }

        ListItem::new(lines)
    } else if annotation.is_empty() {
        let rank_span = Span::styled(format!("[{}]", entry.rank), Style::default().fg(rank_color));
        let command_span = Span::raw(format!(" {}", entry.command));
        ListItem::new(Line::from(vec![rank_span, command_span]))
    } else {
        let mut lines = Vec::new();
        for line in textwrap::wrap(annotation, list_width) {
            lines.push(Line::styled(
                line.to_string(),
                Style::default().fg(Color::Yellow),
            ));
        }
        let rank_span = Span::styled(format!("[{}]", entry.rank), Style::default().fg(rank_color));
        let command_span = Span::raw(format!(" {}", entry.command));
        lines.push(Line::from(vec![rank_span, command_span]));
        ListItem::new(lines)
    }
}

fn render_snippet_layout(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[0]);

    let items: Vec<ListItem> = app
        .snippets
        .iter()
        .map(|snippet| {
            let scope_color = match snippet.scope {
                SnippetScope::Project => Color::Green,
                SnippetScope::Global => Color::Cyan,
            };
            let header = Line::from(vec![
                Span::styled(
                    format!("[{}] ", snippet.scope),
                    Style::default().fg(scope_color),
                ),
                Span::styled(
                    snippet.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]);

            let detail = snippet
                .tags
                .as_ref()
                .map(|tags| Line::styled(tags.clone(), Style::default().fg(Color::DarkGray)))
                .unwrap_or_else(|| Line::raw(snippet.command.clone()));

            ListItem::new(vec![header, detail])
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Snippets"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, content_chunks[0], &mut app.snippet_list_state);

    let detail_text = if let Some(snippet) = app.selected_snippet() {
        format!(
            "Name: {}\nScope: {}\nTags: {}\nUpdated: {}\n\n{}\n\n{}",
            snippet.name,
            snippet.scope,
            snippet.tags.as_deref().unwrap_or("-"),
            snippet.updated_at,
            snippet.description.as_deref().unwrap_or("No description"),
            snippet.command
        )
    } else {
        "No snippet selected".to_string()
    };

    let detail = Paragraph::new(detail_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Snippet Details"),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(detail, content_chunks[1]);

    let status_text = format_status_line(
        "j/k: Navigate | d: Delete | s: Send to REPL | q: Back",
        app.status_message.as_deref(),
    );
    let status =
        Paragraph::new(status_text).style(Style::default().bg(Color::Blue).fg(Color::White));
    f.render_widget(status, chunks[1]);
}

fn render_fullscreen_output(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    if let Some(entry) = app.selected_entry() {
        let paragraph = Paragraph::new(app.current_output_cache.as_deref().unwrap_or("Loading..."))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Output: {}", entry.command))
                    .style(Style::default().bg(Color::Reset)),
            )
            .style(Style::default().bg(Color::Reset))
            .wrap(Wrap { trim: false })
            .scroll((app.fullscreen_scroll, 0));
        f.render_widget(paragraph, chunks[0]);
    } else {
        let paragraph = Paragraph::new("No command selected")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Output")
                    .style(Style::default().bg(Color::Reset)),
            )
            .style(Style::default().bg(Color::Reset));
        f.render_widget(paragraph, chunks[0]);
    }

    let status = Paragraph::new("Enter: Back | q: Quit")
        .style(Style::default().bg(Color::Blue).fg(Color::White));
    f.render_widget(status, chunks[1]);
}

fn render_annotation_popup(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area);
    f.render_widget(&app.annotation_input, area);
}

fn render_create_snippet_popup(f: &mut Frame, app: &mut App) {
    let Some(form) = app.snippet_form.as_ref() else {
        return;
    };

    let area = centered_rect(70, 50, f.area());
    f.render_widget(Clear, area);

    let field_style = |active: bool| {
        if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        }
    };

    let scope_text = match form.scope {
        SnippetScope::Project => "Project",
        SnippetScope::Global => "Global",
    };

    let text = vec![
        Line::styled(
            "Create Snippet",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Command: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(form.command.clone()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Name: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                form.name.clone(),
                field_style(form.active_field == SnippetField::Name),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Description: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                form.description.clone(),
                field_style(form.active_field == SnippetField::Description),
            ),
        ]),
        Line::from(vec![
            Span::styled("Scope: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                scope_text,
                field_style(form.active_field == SnippetField::Scope),
            ),
        ]),
        Line::from(vec![
            Span::styled("Tags: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                form.tags.clone(),
                field_style(form.active_field == SnippetField::Tags),
            ),
        ]),
        Line::raw(""),
        Line::styled(
            "Tab/Shift+Tab: Move | Left/Right: Toggle Scope | Enter: Save | Esc: Cancel",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Snippet"))
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn render_confirmation_popup(f: &mut Frame, title: &str, body: &str, color: Color) {
    let area = centered_rect(40, 10, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(color));

    let paragraph = Paragraph::new(body)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn handle_text_input(value: &mut String, key: KeyEvent) {
    match key.code {
        KeyCode::Backspace => {
            value.pop();
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            value.push(c);
        }
        _ => {}
    }
}

fn trimmed_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_status_line(base: &str, message: Option<&str>) -> String {
    match message {
        Some(message) => format!("{base} | {message}"),
        None => base.to_string(),
    }
}

/// Process carriage returns (`\r`) the way a terminal would: text after `\r`
/// overwrites from the beginning of the line. This collapses progress bar
/// output into the final state of each line.
fn process_carriage_returns(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            if !line.contains('\r') {
                return line.to_string();
            }
            line.rsplit('\r')
                .find(|segment| !segment.is_empty())
                .unwrap_or("")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
