use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};
use ropey::Rope;
use std::{error::Error, io};

enum InputMode {
    Command,
    Log,
}

#[derive(Clone, Copy)]
enum SelectionOrigin {
    Right,
    Left,
}

#[derive(Clone, Copy)]
enum Selection {
    Single(usize),
    Region(usize, usize, SelectionOrigin),
}

/// App holds the state of the application
struct App {
    /// Current value of the input box
    command_buffer: Rope,
    /// Position of cursor in the editor area.
    cursor_selection: Selection,
    /// Current input mode
    input_mode: InputMode,
    /// History of recorded messages
    messages: Vec<String>,

    items: Vec<Vec<&'static str>>,
}

impl Default for App {
    fn default() -> App {
        App {
            command_buffer: Rope::new(),
            input_mode: InputMode::Log,
            messages: Vec::new(),
            cursor_selection: Selection::Single(0),

            items: vec![
                vec!["Row11", "Row12", "Row13"],
                vec!["Row21", "Row22", "Row23"],
                vec!["Row31", "Row32", "Row33"],
                vec!["Row41", "Row42", "Row43"],
                vec!["Row51", "Row52", "Row53"],
                vec![
                    "Row61 asdf asdf asdf asdsdafsdafsdafsdfa sdfasd",
                    "Row62\nTest",
                    "Row63",
                ],
                vec!["Row71", "Row72", "Row73"],
                vec!["Row81", "Row82", "Row83"],
                vec!["Row91", "Row92", "Row93"],
                vec!["Row101", "Row102", "Row103"],
                vec!["Row111", "Row112", "Row113"],
                vec!["Row121", "Row122", "Row123"],
                vec!["Row131", "Row132", "Row133"],
                vec![
                    "Row141",
                    "Row142 asdf asdf asdf asdsdafsdafsdafsdfa sdfasd",
                    "Row143",
                ],
                vec!["Row151", "Row152", "Row153"],
                vec!["Row161", "Row162", "Row163"],
                vec!["Row171", "Row172", "Row173"],
                vec!["Row181", "Row182", "Row183"],
                vec!["Row191", "Row192", "Row193"],
            ],
        }
    }
}

impl App {
    fn move_cursor_left(&mut self, shifted: bool) {
        self.cursor_selection = match self.cursor_selection {
            Selection::Single(i) => {
                if shifted && i > 0 {
                    Selection::Region(i.saturating_sub(1), i, SelectionOrigin::Left)
                } else {
                    Selection::Single(i.saturating_sub(1))
                }
            }
            Selection::Region(start, end, dir) => {
                if shifted {
                    match dir {
                        SelectionOrigin::Right => {
                            if start == end.saturating_sub(1) {
                                Selection::Single(start)
                            } else {
                                Selection::Region(start, end.saturating_sub(1), dir)
                            }
                        },
                        SelectionOrigin::Left => {
                            Selection::Region(start.saturating_sub(1), end, dir)
                        },
                    }
                } else {
                    Selection::Single(start)
                }
            }
        }
    }

    fn clamped(&self, i: usize) -> usize {
        i.clamp(0, self.command_buffer.len_chars())
    }

    fn move_cursor_right(&mut self, shifted: bool) {
        self.cursor_selection = match self.cursor_selection {
            Selection::Single(i) => {
                if shifted && i < self.command_buffer.len_chars() {
                    Selection::Region(
                        i,
                        self.clamped(i.saturating_add(1)),
                        SelectionOrigin::Right,
                    )
                } else {
                    Selection::Single(self.clamped(i.saturating_add(1)))
                }
            }
            Selection::Region(start, end, dir) => {
                if shifted {
                    match dir {
                        SelectionOrigin::Right => {
                            Selection::Region(start, self.clamped(end.saturating_add(1)), dir)
                        },
                        SelectionOrigin::Left => {
                            if start.saturating_add(1) == end {
                                Selection::Single(end)
                            } else {
                                Selection::Region(self.clamped(start.saturating_add(1)), end, dir)
                            }
                        },
                    }
                } else {
                    Selection::Single(end)
                }
            }
        }
    }

    fn enter_char(&mut self, new_char: char) {
        match self.cursor_selection {
            Selection::Single(i) => {
                self.command_buffer.insert_char(i, new_char);
                self.move_cursor_right(false)
            }
            Selection::Region(_, _, _) => {
                self.delete();
                self.enter_char(new_char)
            },
        }
    }

    fn delete(&mut self) -> bool {
        match self.cursor_selection {
            Selection::Single(i) => {
                if i == 0 {
                    return self.command_buffer.len_chars() != 0;
                }
                self.command_buffer.remove(i - 1..i);
                self.move_cursor_left(false)
            }
            Selection::Region(start, end, _) => {
                self.command_buffer.remove(start..end);
                self.move_cursor_left(false);
            },
        }
        true
    }

    fn submit_message(&mut self) {
        self.messages.push(self.command_buffer.to_string());
        self.command_buffer.remove(..);
        self.cursor_selection = Selection::Single(0);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let app = App::default();
    let res = run_app(&mut terminal, app);

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{err:?}");
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &app))?;

        if let Event::Key(key) = event::read()? {
            match app.input_mode {
                InputMode::Log => match key.code {
                    KeyCode::Char(':') => {
                        app.input_mode = InputMode::Command;
                    }
                    KeyCode::Esc => {
                        return Ok(());
                    }
                    _ => {}
                },
                InputMode::Command if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Enter => app.submit_message(),
                    KeyCode::Char(to_insert) => {
                        app.enter_char(to_insert);
                    }
                    KeyCode::Backspace => {
                        if !app.delete() {
                            app.input_mode = InputMode::Log;
                        }
                    }
                    KeyCode::Left => {
                        app.move_cursor_left(key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Right => {
                        app.move_cursor_right(key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Log;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let overall_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.size());

    let command_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(overall_chunks[1]);

    let input = Paragraph::new({
        let mut v = Vec::new();

        match app.cursor_selection {
            Selection::Single(_) => v.push(Span::from(app.command_buffer.slice(..))),
            Selection::Region(start, end, _) => {
                let before = app.command_buffer.slice(..start);
                let selected = app.command_buffer.slice(start..end);
                let after = app.command_buffer.slice(end..);

                v.push(Span::from(before));
                v.push(Span::from(selected).black().on_blue().bold());
                v.push(Span::from(after));
            }
        }

        Line::from(v)
    }).style(match app.input_mode {
        InputMode::Log => Style::default(),
        InputMode::Command => Style::default().fg(Color::Yellow),
    });
    f.render_widget(input, command_chunks[1]);
    match app.input_mode {
        InputMode::Log => {}
        InputMode::Command => {
            f.render_widget(Paragraph::new(":"), command_chunks[0]);

            match app.cursor_selection {
                Selection::Single(i) => {
                    f.set_cursor(command_chunks[1].x + i as u16, command_chunks[1].y)
                }
                Selection::Region(start, end, dir) => {
                    let x = match dir {
                        SelectionOrigin::Right => end,
                        SelectionOrigin::Left => start,
                    };
                    f.set_cursor(command_chunks[1].x + x as u16, command_chunks[1].y)
                }
            }
        }
    }

    let rows = app.items.iter().map(|item| {
        let height = item
            .iter()
            .map(|content| content.chars().filter(|c| *c == '\n').count())
            .max()
            .unwrap_or(0)
            + 1;
        let cells = item.iter().map(|c| Cell::from(*c));
        Row::new(cells).height(height as u16).on_blue()
    });
    let t = Table::new(rows).widths(&[
        Constraint::Percentage(10),
        Constraint::Max(30),
        Constraint::Min(10),
    ]);
    f.render_widget(t, overall_chunks[0]);
}
