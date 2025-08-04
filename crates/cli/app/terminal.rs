use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use anyhow::Result;
use std::ops::{Deref, DerefMut};

type Backend = ratatui::backend::CrosstermBackend<std::io::StdoutLock<'static>>;
pub type Terminal = ratatui::Terminal<Backend>;

pub struct TerminalState {
    inner: Terminal,
    pub mouse_capture: bool,
    entered: bool,
}

impl Deref for TerminalState {
    type Target = Terminal;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TerminalState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        self.exit_terminal()
            .expect("exiting terminal should not error")
    }
}

impl TerminalState {
    pub fn new(term: Terminal) -> Self {
        TerminalState {
            inner: term,
            mouse_capture: true,
            entered: false,
        }
    }

    pub fn enter_terminal(&mut self) -> Result<()> {
        assert!(!self.entered);
        enable_raw_mode()?;
        crossterm::execute!(
            self.inner.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;
        self.entered = true;
        Ok(())
    }

    pub fn exit_terminal(&mut self) -> Result<()> {
        assert!(self.entered);
        disable_raw_mode()?;
        if self.mouse_capture {
            crossterm::execute!(
                self.inner.backend_mut(),
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen,
            )?;
        } else {
            crossterm::execute!(
                self.inner.backend_mut(),
                DisableBracketedPaste,
                LeaveAlternateScreen,
            )?;
        }
        self.entered = false;
        Ok(())
    }

    pub fn toggle_mouse_capture(&mut self) -> Result<()> {
        self.mouse_capture = !self.mouse_capture;
        if self.mouse_capture {
            crossterm::execute!(self.inner.backend_mut(), EnableMouseCapture)?;
        } else {
            crossterm::execute!(self.inner.backend_mut(), DisableMouseCapture)?;
        }
        Ok(())
    }

    pub fn clear(&mut self) -> Result<()> {
        Ok(self.inner.clear()?)
    }
}
