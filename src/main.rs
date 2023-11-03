mod ui;

use bvr::file::ShardedFile;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};
use std::{error::Error, io, time::Duration};
use ui::{
    command::{CommandApp, Cursor, CursorJump, CursorMovement, SelectionOrigin},
    viewer::Viewer,
};

pub type Backend<'a> = ratatui::backend::CrosstermBackend<std::io::StdoutLock<'a>>;
pub type Terminal<'a> = ratatui::Terminal<Backend<'a>>;

#[derive(PartialEq, Clone, Copy)]
enum InputMode {
    Command,
    Viewer,
    Select,
}

/// App holds the state of the application
struct App {
    command: CommandApp,
    viewer: Viewer,
    /// Current input mode
    input_mode: InputMode,
    _rt: tokio::runtime::Runtime,
}

impl App {
    fn new(rt: tokio::runtime::Runtime) -> Self {
        let file = rt
            .block_on(tokio::fs::File::open("./log_generated.log"))
            .unwrap();
        Self {
            input_mode: InputMode::Viewer,
            command: CommandApp::new(),
            viewer: Viewer::new(rt.block_on(ShardedFile::new(file, 25)).unwrap()),
            _rt: rt,
        }
    }

    fn run_app(&mut self, terminal: &mut Terminal) -> io::Result<()> {
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;

        loop {
            terminal.draw(|f| self.ui(f))?;

            if !event::poll(Duration::from_secs_f64(1.0 / 60.0))? {
                continue;
            }
            match event::read()? {
                Event::Mouse(mouse) => match mouse.kind {
                    event::MouseEventKind::ScrollDown => {
                        self.viewer.viewport_mut().move_view_down(2);
                    }
                    event::MouseEventKind::ScrollUp => {
                        self.viewer.viewport_mut().move_view_up(2);
                    }
                    _ => (),
                },
                Event::Paste(paste) => {
                    self.command.enter_str(&paste);
                }
                Event::Key(key) => match self.input_mode {
                    InputMode::Viewer => match key.code {
                        KeyCode::Char(':') => {
                            self.input_mode = InputMode::Command;
                        }
                        KeyCode::Char('i') => {
                            self.input_mode = InputMode::Select;
                        }
                        KeyCode::Esc => {
                            break;
                        }
                        KeyCode::Up => self.viewer.viewport_mut().move_view_up(1),
                        KeyCode::Down => self.viewer.viewport_mut().move_view_down(1),
                        _ => {}
                    },
                    InputMode::Select => match key.code {
                        KeyCode::Char(':') => {
                            self.input_mode = InputMode::Command;
                        }
                        KeyCode::Esc => {
                            self.input_mode = InputMode::Viewer;
                        }
                        KeyCode::Up => self.viewer.viewport_mut().move_select_up(1),
                        KeyCode::Down => self.viewer.viewport_mut().move_select_down(1),
                        _ => {}
                    },
                    InputMode::Command if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Esc => {
                            self.input_mode = InputMode::Viewer;
                        }
                        KeyCode::Enter => {
                            if self.command.submit() == "q" {
                                break;
                            }
                        }
                        KeyCode::Left => {
                            self.command.move_left(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                if key.modifiers.contains(KeyModifiers::ALT) {
                                    CursorJump::Word
                                } else {
                                    CursorJump::None
                                },
                            ));
                        }
                        KeyCode::Right => {
                            self.command.move_right(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                if key.modifiers.contains(KeyModifiers::ALT) {
                                    CursorJump::Word
                                } else {
                                    CursorJump::None
                                },
                            ));
                        }
                        KeyCode::Home => {
                            self.command.move_left(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                CursorJump::Boundary,
                            ));
                        }
                        KeyCode::End => {
                            self.command.move_right(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                CursorJump::Boundary,
                            ));
                        }
                        KeyCode::Backspace => {
                            if !self.command.delete() {
                                self.input_mode = InputMode::Viewer;
                            }
                        }
                        KeyCode::Char(to_insert) => match to_insert {
                            'b' if key.modifiers.contains(KeyModifiers::ALT) => {
                                self.command.move_left(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Word,
                                ));
                            }
                            'f' if key.modifiers.contains(KeyModifiers::ALT) => {
                                self.command.move_right(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Word,
                                ));
                            }
                            'a' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.command.move_left(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Boundary,
                                ));
                            }
                            'e' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.command.move_right(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Boundary,
                                ));
                            }
                            _ => self.command.enter_char(to_insert),
                        },
                        _ => {}
                    },
                    _ => {}
                },
                _ => (),
            }
        }

        // restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen,
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    fn ui(&mut self, f: &mut Frame) {
        let overall_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(f.size());

        let mut cursor = None;
        f.render_widget(
            CommandWidget {
                active: self.input_mode == InputMode::Command,
                inner: &self.command,
                cursor: &mut cursor,
            },
            overall_chunks[2],
        );
        f.render_widget(
            StatusWidget {
                input_mode: self.input_mode,
                progress: self.viewer.file().progress(),
            },
            overall_chunks[1],
        );

        if let Some((x, y)) = cursor {
            f.set_cursor(x, y);
        }

        self.viewer
            .viewport_mut()
            .fit_view(overall_chunks[0].height as usize);

        let view = self.viewer.update_and_view();
        let rows = view.iter().map(|(ln, data)| {
            let mut row = Row::new([Cell::from((ln + 1).to_string()), Cell::from(data.as_str())]);

            if *ln == self.viewer.viewport_mut().current() {
                row = row.on_dark_gray();
            }

            row.height(1)
        });
        // Wait til https://github.com/ratatui-org/ratatui/issues/537 is fixed
        let t = Table::new(rows).widths(&[Constraint::Percentage(5), Constraint::Percentage(95)]);

        f.render_widget(t, overall_chunks[0]);
    }
}

struct StatusWidget {
    input_mode: InputMode,
    progress: f64,
}

impl Widget for StatusWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let command_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(9)])
            .split(area);

        Paragraph::new(Span::from(if self.progress > 1.0 {
            format!("Indexing complete")
        } else {
            format!("{:.2}%", self.progress * 100.0)
        }))
        .dark_gray()
        .on_black()
        .render(command_chunks[0], buf);

        Paragraph::new(Span::from(match self.input_mode {
            InputMode::Command => "COMMAND",
            InputMode::Viewer => "VIEWER",
            InputMode::Select => "SELECT",
        }))
        .alignment(Alignment::Center)
        .on_blue()
        .render(command_chunks[1], buf);
    }
}

struct CommandWidget<'a> {
    inner: &'a CommandApp,
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
                Cursor::Singleton(_) => v.push(Span::from(self.inner.buf())),
                Cursor::Selection(start, end, _) => {
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
                    Cursor::Singleton(i) => {
                        *self.cursor = Some((command_chunks[1].x + i as u16, command_chunks[1].y));
                    }
                    Cursor::Selection(start, end, dir) => {
                        let x = match dir {
                            SelectionOrigin::Right => end,
                            SelectionOrigin::Left => start,
                        };
                        *self.cursor = Some((command_chunks[1].x + x as u16, command_chunks[1].y));
                    }
                }
            }
        }
        input.render(command_chunks[1], buf);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let stdout = io::stdout().lock();
    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::new(backend)?;

    let rt = tokio::runtime::Runtime::new().unwrap();

    // create app and run it
    let mut app = App::new(rt);
    let res = app.run_app(&mut terminal);

    if let Err(err) = res {
        println!("{err:?}");
    }

    Ok(())
}
