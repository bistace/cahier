use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap, Clear},
};
use std::io::{self, Stdout};

use crate::db::{Database, Entry, Direction as DbDirection};

pub fn run(db: Database) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    let app_result = run_app(&mut terminal, db);
    
    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    app_result
}

struct App {
    db: Database,
    entries: Vec<Entry>,
    list_state: ListState,
    should_quit: bool,
    show_output_fullscreen: bool,
    input_mode: InputMode,
    input_buffer: String,
    fullscreen_scroll: u16,
    // For popup messages or confirmation if needed, sticking to simple for now
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    EditingAnnotation,
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, db: Database) -> Result<()> {
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
                        },
                        KeyCode::Down if app.show_output_fullscreen => {
                            app.scroll_fullscreen_down();
                            // Clear buffer when scrolling to avoid artifacts
                            terminal.clear()?;
                        },
                        KeyCode::Char('q') => {
                            if app.show_output_fullscreen {
                                app.toggle_output_fullscreen();
                                terminal.clear()?;
                            } else {
                                app.should_quit = true;
                            }
                        },
                        KeyCode::Char('j') | KeyCode::Down => app.next(),
                        KeyCode::Char('k') | KeyCode::Up => app.previous(),
                        KeyCode::Char('J') => app.move_entry(DbDirection::Down)?,
                        KeyCode::Char('K') => app.move_entry(DbDirection::Up)?,
                        KeyCode::Char('d') => app.delete_entry()?,
                        KeyCode::Char('a') => app.start_editing_annotation(),
                        KeyCode::Enter => {
                            app.toggle_output_fullscreen();
                            // Clear terminal buffer when entering fullscreen to remove artifacts
                            terminal.clear()?;
                        },
                        _ => {}
                    },
                    InputMode::EditingAnnotation => match key.code {
                        KeyCode::Enter => app.save_annotation()?,
                        KeyCode::Esc => app.cancel_editing_annotation(),
                        KeyCode::Char(c) => {
                            app.input_buffer.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_buffer.pop();
                        }
                        _ => {}
                    }
                }
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

impl App {
    fn new(db: Database) -> Result<Self> {
        let entries = db.get_all_entries_ordered()?;
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(entries.len() - 1)); // Select last by default
        }

        Ok(Self {
            db,
            entries,
            list_state,
            should_quit: false,
            show_output_fullscreen: false,
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            fullscreen_scroll: 0,
        })
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
    }

    fn move_entry(&mut self, direction: DbDirection) -> Result<()> {
        if let Some(i) = self.list_state.selected() {
            let id = self.entries[i].id;
            self.db.move_entry(id, direction)?;
            self.refresh_entries()?;
            
            // Try to keep selection on the moved item
            // If moved down, index increases. If moved up, index decreases.
            // But since we reload, we just need to find the new index of the same ID 
            // or just adjust index if swap happened.
            // Simpler: just adjust index if possible.
            let new_i = match direction {
                DbDirection::Up => if i > 0 { i - 1 } else { i },
                DbDirection::Down => if i < self.entries.len() - 1 { i + 1 } else { i },
            };
            self.list_state.select(Some(new_i));
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
        }
        Ok(())
    }

    fn start_editing_annotation(&mut self) {
        if let Some(i) = self.list_state.selected() {
            self.input_mode = InputMode::EditingAnnotation;
            self.input_buffer = self.entries[i].annotation.clone().unwrap_or_default();
        }
    }

    fn cancel_editing_annotation(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input_buffer.clear();
    }

    fn save_annotation(&mut self) -> Result<()> {
        if let Some(i) = self.list_state.selected() {
            let id = self.entries[i].id;
            self.db.update_annotation(id, self.input_buffer.clone())?;
            self.refresh_entries()?;
            self.input_mode = InputMode::Normal;
            self.input_buffer.clear();
        }
        Ok(())
    }

    fn toggle_output_fullscreen(&mut self) {
        self.show_output_fullscreen = !self.show_output_fullscreen;
        self.fullscreen_scroll = 0;
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
        self.entries = self.db.get_all_entries_ordered()?;
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
}

fn render_main_layout(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1), // Status bar
        ])
        .split(f.area());

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(chunks[0]);

    // List
    let items: Vec<ListItem> = app.entries
        .iter()
        .map(|e| {
            let annotation = e.annotation.as_deref().unwrap_or("");
            let content = if annotation.is_empty() {
                format!("[{}] {}", e.id, e.command)
            } else {
                format!("[{}] {} ({})", e.id, e.command, annotation)
            };
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("History"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, content_chunks[0], &mut app.list_state);

    // Preview
    if let Some(i) = app.list_state.selected() {
        if let Some(entry) = app.entries.get(i) {
            let output_text = &entry.output;
            let p = Paragraph::new(output_text.as_str())
                .block(Block::default().borders(Borders::ALL).title("Output Preview"))
                .wrap(Wrap { trim: true });
            f.render_widget(p, content_chunks[1]);
        }
    } else {
        let p = Paragraph::new("No command selected")
            .block(Block::default().borders(Borders::ALL).title("Output Preview"));
        f.render_widget(p, content_chunks[1]);
    }
    
    // Status bar
    let status_text = "j/k: Navigate | J/K: Move | d: Delete | a: Annotate | Enter: Fullscreen | q: Quit";
    let status = Paragraph::new(status_text)
        .style(Style::default().bg(Color::Blue).fg(Color::White));
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
            let output_text = &entry.output;
            let p = Paragraph::new(output_text.as_str())
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
                .block(Block::default().borders(Borders::ALL).title("Output").style(Style::default().bg(Color::Reset)))
                .style(Style::default().bg(Color::Reset));
            f.render_widget(p, chunks[0]);
        }
    } else {
        let p = Paragraph::new("No command selected")
            .block(Block::default().borders(Borders::ALL).title("Output").style(Style::default().bg(Color::Reset)))
            .style(Style::default().bg(Color::Reset));
        f.render_widget(p, chunks[0]);
    }
    
    // Status bar for fullscreen
    let status_text = "Enter: Back | q: Quit";
    let status = Paragraph::new(status_text)
        .style(Style::default().bg(Color::Blue).fg(Color::White));
    f.render_widget(status, chunks[1]);
}

fn render_annotation_popup(f: &mut Frame, app: &mut App) {
    let block = Block::default().borders(Borders::ALL).title("Edit Annotation");
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area); // Clear background
    
    let p = Paragraph::new(app.input_buffer.as_str())
        .block(block)
        .style(Style::default().fg(Color::Yellow));
    
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

