mod command;
mod viewer;

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
    Viewer,
}

/// App holds the state of the application
struct App {
    command: command::CommandApp,
    viewer: viewer::Viewer,
    /// Current input mode
    input_mode: InputMode,
}

impl App {
    fn new() -> Self {
        Self {
            input_mode: InputMode::Viewer,

            command: command::CommandApp::new(),
            viewer: viewer::Viewer::new(),
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
        terminal.draw(|f| ui(f, &mut app))?;

        match event::read()? {
            Event::Mouse(mouse) => {
                match mouse.kind {
                    // event::MouseEventKind::Down(_) => todo!(),
                    // event::MouseEventKind::Up(_) => todo!(),
                    // event::MouseEventKind::Moved => todo!(),
                    event::MouseEventKind::ScrollDown => {
                        app.viewer.viewport_mut().move_down();
                    },
                    event::MouseEventKind::ScrollUp => {
                        app.viewer.viewport_mut().move_up();
                    },
                    // event::MouseEventKind::ScrollLeft => todo!(),
                    // event::MouseEventKind::ScrollRight => todo!(),
                    _ => ()
                }
            }
            Event::Key(key) => {
                match app.input_mode {
                    InputMode::Viewer => match key.code {
                        KeyCode::Char(':') => {
                            app.input_mode = InputMode::Command;
                        }
                        KeyCode::Esc => {
                            return Ok(());
                        }
                        KeyCode::Up => app.viewer.viewport_mut().move_up(),
                        KeyCode::Down => app.viewer.viewport_mut().move_down(),
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
                                app.input_mode = InputMode::Viewer;
                            }
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Viewer;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            _ => (),
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

            match *self.inner.cursor() {
                command::Cursor::Singleton(_) => v.push(Span::from(self.inner.buf())),
                command::Cursor::Selection(start, end, _) => {
                    v.push(Span::from(&self.inner.buf()[..start]));
                    v.push(Span::from(&self.inner.buf()[start..end]).on_blue());
                    v.push(Span::from(&self.inner.buf()[end..]));
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
                match *self.inner.cursor() {
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

fn ui(f: &mut Frame, app: &mut App) {
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

    app.viewer
        .viewport_mut()
        .fit_view(overall_chunks[0].height as usize);

    let rows = app.viewer.viewport().line_range().map(|line_number| {
        let ln = (line_number + 1).to_string();

        let mut row = if line_number < app.viewer.viewport().max_height() {
            Row::new([Cell::from(ln), Cell::from("hi")]).height(1)
        } else {
            Row::new([] as [Cell; 0]).height(1)
        };

        if app.viewer.viewport_mut().current() == line_number {
            row = row.on_blue();
        }

        row
    });
    let t = Table::new(rows).widths(&[Constraint::Max(8), Constraint::Min(1)]);
    f.render_widget(t, overall_chunks[0]);
}
