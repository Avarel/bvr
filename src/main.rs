mod app;
mod command;
mod tui;
mod viewer;

use anyhow::Result;

fn main() -> Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(tokio_main())
}

async fn tokio_main() -> Result<()> {
    let mut app = app::App::new().await?;
    app.run().await?;
    Ok(())
}

// fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
//     loop {
//         terminal.draw(|f| ui(f, &mut app))?;

//         match event::read()? {
//             Event::Mouse(mouse) => match mouse.kind {
//                 event::MouseEventKind::ScrollDown => {
//                     app.viewer.viewport_mut().move_down();
//                 }
//                 event::MouseEventKind::ScrollUp => {
//                     app.viewer.viewport_mut().move_up();
//                 }
//                 _ => (),
//             },
//             Event::Key(key) => match app.input_mode {
//                 InputMode::Viewer => match key.code {
//                     KeyCode::Char(':') => {
//                         app.input_mode = InputMode::Command;
//                     }
//                     KeyCode::Esc => {
//                         return Ok(());
//                     }
//                     KeyCode::Up => app.viewer.viewport_mut().move_up(),
//                     KeyCode::Down => app.viewer.viewport_mut().move_down(),
//                     _ => {}
//                 },
//                 InputMode::Command if key.kind == KeyEventKind::Press => match key.code {
//                     KeyCode::Enter => {
//                         if app.command.submit() == "q" {
//                             return Ok(());
//                         }
//                     }
//                     KeyCode::Left => {
//                         app.command.move_left(command::CursorMovement::new(
//                             key.modifiers.contains(KeyModifiers::SHIFT),
//                             if key.modifiers.contains(KeyModifiers::ALT) {
//                                 command::CursorJump::Word
//                             } else {
//                                 command::CursorJump::None
//                             },
//                         ));
//                     }
//                     KeyCode::Right => {
//                         app.command.move_right(command::CursorMovement::new(
//                             key.modifiers.contains(KeyModifiers::SHIFT),
//                             if key.modifiers.contains(KeyModifiers::ALT) {
//                                 command::CursorJump::Word
//                             } else {
//                                 command::CursorJump::None
//                             },
//                         ));
//                     }
//                     KeyCode::Home => {
//                         app.command.move_left(command::CursorMovement::new(
//                             key.modifiers.contains(KeyModifiers::SHIFT),
//                             command::CursorJump::Boundary,
//                         ));
//                     }
//                     KeyCode::End => {
//                         app.command.move_right(command::CursorMovement::new(
//                             key.modifiers.contains(KeyModifiers::SHIFT),
//                             command::CursorJump::Boundary,
//                         ));
//                     }
//                     KeyCode::Char(to_insert) => match to_insert {
//                         'b' if key.modifiers.contains(KeyModifiers::ALT) => {
//                             app.command.move_left(command::CursorMovement::new(
//                                 key.modifiers.contains(KeyModifiers::SHIFT),
//                                 command::CursorJump::Word,
//                             ));
//                         }
//                         'f' if key.modifiers.contains(KeyModifiers::ALT) => {
//                             app.command.move_right(command::CursorMovement::new(
//                                 key.modifiers.contains(KeyModifiers::SHIFT),
//                                 command::CursorJump::Word,
//                             ));
//                         }
//                         'a' if key.modifiers.contains(KeyModifiers::CONTROL) => {
//                             app.command.move_left(command::CursorMovement::new(
//                                 key.modifiers.contains(KeyModifiers::SHIFT),
//                                 command::CursorJump::Boundary,
//                             ));
//                         }
//                         'e' if key.modifiers.contains(KeyModifiers::CONTROL) => {
//                             app.command.move_right(command::CursorMovement::new(
//                                 key.modifiers.contains(KeyModifiers::SHIFT),
//                                 command::CursorJump::Boundary,
//                             ));
//                         }
//                         _ => app.command.enter_char(to_insert),
//                     },
//                     KeyCode::Backspace => {
//                         if !app.command.delete() {
//                             app.input_mode = InputMode::Viewer;
//                         }
//                     }
//                     KeyCode::Esc => {
//                         app.input_mode = InputMode::Viewer;
//                     }
//                     _ => {}
//                 },
//                 _ => {}
//             },
//             _ => (),
//         }
//     }
// }