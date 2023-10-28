mod command;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};
use std::{error::Error, io};

#[derive(PartialEq)]
enum InputMode {
    Command,
    Log,
}

/// App holds the state of the application
struct App {
    command: command::CommandApp,
    /// Current input mode
    input_mode: InputMode,

    items: Vec<Vec<&'static str>>,
}

impl App {
    fn new() -> Self {
        Self {
            input_mode: InputMode::Log,

            command: command::CommandApp::new(),

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

fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let stdout = io::stdout().lock();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;

    // create app and run it
    let app = App::new();
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
                    KeyCode::Enter => {
                        if app.command.submit() == "q" {
                            return Ok(());
                        }
                    }
                    KeyCode::Left => {
                        app.command.move_left(command::CursorMovement::new(
                            key.modifiers.contains(KeyModifiers::SHIFT),
                            if key.modifiers.contains(KeyModifiers::ALT) {
                                command::CursorJump::Word
                            } else {
                                command::CursorJump::None
                            },
                        ));
                    }
                    KeyCode::Right => {
                        app.command.move_right(command::CursorMovement::new(
                            key.modifiers.contains(KeyModifiers::SHIFT),
                            if key.modifiers.contains(KeyModifiers::ALT) {
                                command::CursorJump::Word
                            } else {
                                command::CursorJump::None
                            },
                        ));
                    }
                    KeyCode::Home => {
                        app.command.move_left(command::CursorMovement::new(
                            key.modifiers.contains(KeyModifiers::SHIFT),
                            command::CursorJump::Boundary,
                        ));
                    }
                    KeyCode::End => {
                        app.command.move_right(command::CursorMovement::new(
                            key.modifiers.contains(KeyModifiers::SHIFT),
                            command::CursorJump::Boundary,
                        ));
                    }
                    KeyCode::Char(to_insert) => match to_insert {
                        'b' if key.modifiers.contains(KeyModifiers::ALT) => {
                            app.command.move_left(command::CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                command::CursorJump::Word,
                            ));
                        }
                        'f' if key.modifiers.contains(KeyModifiers::ALT) => {
                            app.command.move_right(command::CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                command::CursorJump::Word,
                            ));
                        }
                        'a' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.command.move_left(command::CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                command::CursorJump::Boundary,
                            ));
                        }
                        'e' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.command.move_right(command::CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                command::CursorJump::Boundary,
                            ));
                        }
                        _ => app.command.enter_char(to_insert),
                    },
                    KeyCode::Backspace => {
                        if !app.command.delete() {
                            app.input_mode = InputMode::Log;
                        }
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

struct CommandWidget<'a> {
    inner: &'a command::CommandApp,
    cursor: &'a mut Option<(u16, u16)>,
    active: bool,
}

impl Widget for CommandWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let command_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let input = Paragraph::new({
            let mut v = Vec::new();

            match self.inner.cursor {
                command::Cursor::Singleton(_) => v.push(Span::from(&self.inner.buf)),
                command::Cursor::Selection(start, end, _) => {
                    v.push(Span::from(&self.inner.buf[..start]));
                    v.push(Span::from(&self.inner.buf[start..end]).on_blue());
                    v.push(Span::from(&self.inner.buf[end..]));
                }
            }

            Line::from(v)
        })
        .style(match self.active {
            false => Style::default(),
            true => Style::default().fg(Color::Yellow),
        });
        match self.active {
            false => {}
            true => {
                Paragraph::new(":").render(command_chunks[0], buf);
                match self.inner.cursor {
                    command::Cursor::Singleton(i) => {
                        *self.cursor = Some((command_chunks[1].x + i as u16, command_chunks[1].y));
                    }
                    command::Cursor::Selection(start, end, dir) => {
                        let x = match dir {
                            command::SelectionOrigin::Right => end,
                            command::SelectionOrigin::Left => start,
                        };
                        *self.cursor = Some((command_chunks[1].x + x as u16, command_chunks[1].y));
                    }
                }
            }
        }
        input.render(command_chunks[1], buf);
    }
}

fn ui(f: &mut Frame, app: &App) {
    let overall_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.size());

    let mut cursor = None;
    f.render_widget(
        CommandWidget {
            active: app.input_mode == InputMode::Command,
            inner: &app.command,
            cursor: &mut cursor,
        },
        overall_chunks[1],
    );

    if let Some((x, y)) = cursor {
        f.set_cursor(x, y);
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
