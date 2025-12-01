use anyhow::Result;
use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io::{self, Stdout};
use tui_textarea::TextArea;

use crate::db::{Database, Direction as DbDirection, EntrySummary};

pub fn run(db: Database) -> Result<Option<String>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_result = run_app(&mut terminal, db);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    app_result
}

struct App<'a> {
    db: Database,
    entries: Vec<EntrySummary>,
    list_state: ListState,
    should_quit: bool,
    show_output_fullscreen: bool,
    input_mode: InputMode,
    input_buffer: TextArea<'a>,
    fullscreen_scroll: u16,
    selected_command: Option<String>,
    current_output_cache: Option<String>,
    preview_collapsed: bool,
    // For popup messages or confirmation if needed, sticking to simple for now
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    EditingAnnotation,
    ConfirmDelete,
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    db: Database,
) -> Result<Option<String>> {
    let mut app = App::new(db)?;

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Up if app.show_output_fullscreen => {
                            app.scroll_fullscreen_up();
                            // Clear buffer when scrolling to avoid artifacts
                            terminal.clear()?;
                        }
                        KeyCode::Down if app.show_output_fullscreen => {
                            app.scroll_fullscreen_down();
                            // Clear buffer when scrolling to avoid artifacts
                            terminal.clear()?;
                        }
                        KeyCode::Char('q') => {
                            if app.show_output_fullscreen {
                                app.toggle_output_fullscreen();
                                terminal.clear()?;
                            } else {
                                app.should_quit = true;
                            }
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.next();
                            terminal.clear()?;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.previous();
                            terminal.clear()?;
                        }
                        KeyCode::Char('J') => app.move_entry(DbDirection::Down)?,
                        KeyCode::Char('K') => app.move_entry(DbDirection::Up)?,
                        KeyCode::Char('d') => {
                            if app.list_state.selected().is_some() {
                                app.input_mode = InputMode::ConfirmDelete;
                            }
                        }
                        KeyCode::Char('a') => app.start_editing_annotation(),
                        KeyCode::Char(' ') => app.insert_separator()?,
                        KeyCode::Char('s') => {
                            if let Some(i) = app.list_state.selected() {
                                if let Some(entry) = app.entries.get(i) {
                                    app.selected_command = Some(entry.command.clone());
                                    app.should_quit = true;
                                }
                            }
                        }
                        KeyCode::Enter => {
                            app.toggle_output_fullscreen();
                            // Clear terminal buffer when entering fullscreen to remove artifacts
                            terminal.clear()?;
                        }
                        KeyCode::Char('p') => {
                            app.toggle_preview_collapsed();
                            terminal.clear()?;
                        }
                        _ => {}
                    },
                    InputMode::EditingAnnotation => match key.code {
                        KeyCode::Esc => app.save_annotation()?,
                        _ => {
                            app.input_buffer.input(key);
                        }
                    },
                    InputMode::ConfirmDelete => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            app.delete_entry()?;
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {}
                    },
                }
            }
        }

        if app.should_quit {
            return Ok(app.selected_command);
        }
    }
}

impl<'a> App<'a> {
    fn new(db: Database) -> Result<Self> {
        let entries = db.get_all_entry_summaries()?;
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(entries.len() - 1)); // Select last by default
        }

        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Edit Annotation"),
        );
        textarea.set_style(Style::default().fg(Color::Yellow));
        textarea.set_cursor_line_style(Style::default());

        let mut app = Self {
            db,
            entries,
            list_state,
            should_quit: false,
            show_output_fullscreen: false,
            input_mode: InputMode::Normal,
            input_buffer: textarea,
            fullscreen_scroll: 0,
            selected_command: None,
            current_output_cache: None,
            preview_collapsed: false,
        };
        app.update_output_cache();
        Ok(app)
    }

    fn update_output_cache(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if let Some(entry) = self.entries.get(i) {
                self.current_output_cache = self.db.get_entry_output(entry.id).ok();
            } else {
                self.current_output_cache = None;
            }
        } else {
            self.current_output_cache = None;
        }
    }

    fn next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.entries.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.update_output_cache();
    }

    fn previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.entries.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.update_output_cache();
    }

    fn move_entry(&mut self, direction: DbDirection) -> Result<()> {
        if let Some(i) = self.list_state.selected() {
            let id = self.entries[i].id;
            self.db.move_entry(id, direction)?;
            self.refresh_entries()?;

            // Try to keep selection on the moved item
            // If moved down, index increases. If moved up, index decreases.
            // But since we reload, we just need to find the new index of the same ID
            // or just adjust index if possible.
            let new_i = match direction {
                DbDirection::Up => {
                    if i > 0 {
                        i - 1
                    } else {
                        i
                    }
                }
                DbDirection::Down => {
                    if i < self.entries.len() - 1 {
                        i + 1
                    } else {
                        i
                    }
                }
            };
            self.list_state.select(Some(new_i));
            self.update_output_cache();
        }
        Ok(())
    }

    fn delete_entry(&mut self) -> Result<()> {
        if let Some(i) = self.list_state.selected() {
            let id = self.entries[i].id;
            self.db.delete_entry(id)?;
            self.refresh_entries()?;
            if i >= self.entries.len() && !self.entries.is_empty() {
                self.list_state.select(Some(self.entries.len() - 1));
            } else if self.entries.is_empty() {
                self.list_state.select(None);
            }
            self.update_output_cache();
        }
        Ok(())
    }

    fn insert_separator(&mut self) -> Result<()> {
        let current_rank = if let Some(i) = self.list_state.selected() {
            if let Some(entry) = self.entries.get(i) {
                entry.rank
            } else {
                // Should not happen if list_state has selection
                0
            }
        } else {
            // If nothing selected (empty list or whatever), we can insert at rank 1
            0
        };

        // Insert after the current selection
        self.db.insert_separator(current_rank + 1)?;
        self.refresh_entries()?;

        // Select the new separator
        if let Some(i) = self.list_state.selected() {
            if i < self.entries.len() - 1 {
                self.list_state.select(Some(i + 1));
            }
        } else if !self.entries.is_empty() {
            self.list_state.select(Some(0));
        }

        self.update_output_cache();
        Ok(())
    }

    fn start_editing_annotation(&mut self) {
        if let Some(i) = self.list_state.selected() {
            self.input_mode = InputMode::EditingAnnotation;
            let content = self.entries[i].annotation.clone().unwrap_or_default();
            self.input_buffer = TextArea::new(content.lines().map(|s| s.to_string()).collect());
            self.input_buffer.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Edit Annotation"),
            );
            self.input_buffer
                .set_style(Style::default().fg(Color::Yellow));
            self.input_buffer.set_cursor_line_style(Style::default());
            // Move cursor to end
            self.input_buffer
                .move_cursor(tui_textarea::CursorMove::Bottom);
            self.input_buffer.move_cursor(tui_textarea::CursorMove::End);
        }
    }

    fn save_annotation(&mut self) -> Result<()> {
        if let Some(i) = self.list_state.selected() {
            let id = self.entries[i].id;
            let content = self.input_buffer.lines().join("\n");
            self.db.update_annotation(id, content)?;
            self.refresh_entries()?;
            self.input_mode = InputMode::Normal;
            // Clear textarea
            self.input_buffer = TextArea::default();
            self.input_buffer.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Edit Annotation"),
            );
            self.input_buffer
                .set_style(Style::default().fg(Color::Yellow));
            self.input_buffer.set_cursor_line_style(Style::default());
            // Re-select same index
            self.list_state.select(Some(i));
        }
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
        // TODO: Add limit check based on content height
        self.fullscreen_scroll += 1;
    }

    fn refresh_entries(&mut self) -> Result<()> {
        self.entries = self.db.get_all_entry_summaries()?;
        Ok(())
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    if app.show_output_fullscreen {
        render_fullscreen_output(f, app);
    } else {
        render_main_layout(f, app);
    }

    if app.input_mode == InputMode::EditingAnnotation {
        render_annotation_popup(f, app);
    }

    if app.input_mode == InputMode::ConfirmDelete {
        render_delete_confirmation(f);
    }
}

fn render_main_layout(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1), // Status bar
        ])
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

    // List
    let list_width = content_chunks[0].width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|e| {
            if e.is_separator {
                let separator = " --- ";
                let content = Line::styled(separator, Style::default().fg(Color::DarkGray));
                return ListItem::new(content);
            }

            let annotation = e.annotation.as_deref().unwrap_or("");
            let id_color = match e.exit_code {
                Some(0) => Color::Blue,
                _ => Color::Red,
            };

            if app.preview_collapsed {
                let id_str = format!("[{}]", e.rank);
                let id_span = Span::styled(id_str.clone(), Style::default().fg(id_color));
                let mut lines = Vec::new();

                // 1. Annotation (if any)
                if !annotation.is_empty() {
                    let wrapped_annotation = textwrap::wrap(annotation, list_width);
                    for line in wrapped_annotation {
                        lines.push(Line::styled(
                            line.to_string(),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                }

                // 2. Command
                // Calculate indent length
                let indent_len = id_str.len() + 1; // +1 for space
                let indent = " ".repeat(indent_len);

                let full_command = format!("{} {}", id_str, e.command);

                // Use textwrap with custom indentation for subsequent lines
                let options = textwrap::Options::new(list_width).subsequent_indent(&indent);

                let wrapped_command = textwrap::wrap(&full_command, &options);

                for (idx, line) in wrapped_command.iter().enumerate() {
                    if idx == 0 {
                        // Reconstruct first line with colored ID
                        let line_str = line.to_string();
                        if line_str.starts_with(&id_str) {
                            let content_part = &line_str[id_str.len()..];
                            lines.push(Line::from(vec![
                                id_span.clone(),
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
            } else {
                if annotation.is_empty() {
                    let id_span =
                        Span::styled(format!("[{}]", e.rank), Style::default().fg(id_color));
                    let command_span = Span::raw(format!(" {}", e.command));
                    ListItem::new(Line::from(vec![id_span, command_span]))
                } else {
                    let mut lines = Vec::new();
                    let wrapped = textwrap::wrap(annotation, list_width);
                    for line in wrapped {
                        lines.push(Line::styled(
                            line.to_string(),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                    let id_span =
                        Span::styled(format!("[{}]", e.rank), Style::default().fg(id_color));
                    let command_span = Span::raw(format!(" {}", e.command));
                    lines.push(Line::from(vec![id_span, command_span]));
                    ListItem::new(lines)
                }
            }
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("History"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, content_chunks[0], &mut app.list_state);

    // Preview
    if !app.preview_collapsed {
        if app.list_state.selected().is_some() {
            let output_text = app.current_output_cache.as_deref().unwrap_or("Loading...");
            let p = Paragraph::new(output_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Output Preview"),
                )
                .wrap(Wrap { trim: true });
            f.render_widget(p, content_chunks[1]);
        } else {
            let p = Paragraph::new("No command selected").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Output Preview"),
            );
            f.render_widget(p, content_chunks[1]);
        }
    }

    // Status bar
    let status_text = "j/k: Navigate | J/K: Move | d: Delete | a: Annotate | Space: Separator | s: Send to REPL | Enter: Fullscreen | p: Toggle Preview | q: Quit";
    let status =
        Paragraph::new(status_text).style(Style::default().bg(Color::Blue).fg(Color::White));
    f.render_widget(status, chunks[1]);
}

fn render_fullscreen_output(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Clear, area); // Explicitly clear the background

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Render output content
    if let Some(i) = app.list_state.selected() {
        if let Some(entry) = app.entries.get(i) {
            let output_text = app.current_output_cache.as_deref().unwrap_or("Loading...");
            let p = Paragraph::new(output_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("Output: {}", entry.command))
                        .style(Style::default().bg(Color::Reset)),
                )
                .style(Style::default().bg(Color::Reset))
                .wrap(Wrap { trim: false })
                .scroll((app.fullscreen_scroll, 0));
            f.render_widget(p, chunks[0]);
        } else {
            // Should not happen if selected index is valid, but fallback
            let p = Paragraph::new("No command selected")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Output")
                        .style(Style::default().bg(Color::Reset)),
                )
                .style(Style::default().bg(Color::Reset));
            f.render_widget(p, chunks[0]);
        }
    } else {
        let p = Paragraph::new("No command selected")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Output")
                    .style(Style::default().bg(Color::Reset)),
            )
            .style(Style::default().bg(Color::Reset));
        f.render_widget(p, chunks[0]);
    }

    // Status bar for fullscreen
    let status_text = "Enter: Back | q: Quit";
    let status =
        Paragraph::new(status_text).style(Style::default().bg(Color::Blue).fg(Color::White));
    f.render_widget(status, chunks[1]);
}

fn render_annotation_popup(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area); // Clear background
    f.render_widget(&app.input_buffer, area);
}

fn render_delete_confirmation(f: &mut Frame) {
    let area = centered_rect(40, 10, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Confirm Deletion")
        .style(Style::default().fg(Color::Red));

    let p = Paragraph::new("Are you sure you want to delete this entry?\n\n(y)es / (n)o")
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
