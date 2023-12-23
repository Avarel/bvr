mod app;
mod components;
mod direction;
mod colors;

use std::{io::IsTerminal, path::PathBuf};

use anyhow::Result;
use app::App;
use ratatui::{prelude::CrosstermBackend, Terminal};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let stdout = std::io::stdout().lock();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    for path in args.files {
        app.open_file(path)?;
    }

    if !std::io::stdin().is_terminal() {
        app.open_stream(Box::new(std::io::stdin()))?;
    }

    app.run_app(&mut terminal)
}
