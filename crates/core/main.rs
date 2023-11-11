mod app;
mod components;
mod direction;

use std::{path::PathBuf, io::IsTerminal};

use anyhow::Result;
use app::App;
use ratatui::{prelude::CrosstermBackend, Terminal};

use clap::Parser;

/// Simple program to greet a person
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

    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut app = App::new(rt);

    for path in args.files {
        app.open_file(path)?;
    }

    if !std::io::stdin().is_terminal() {
        app.open_stream(Box::new(std::io::stdin()))?;
    }

    app.run_app(&mut terminal)
}