mod app;
mod ui;

use std::path::PathBuf;

use anyhow::Result;
use app::App;
use ratatui::{prelude::CrosstermBackend, Terminal};

use clap::Parser;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    file: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let stdout = std::io::stdout().lock();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut app = App::new(rt);

    if let Some(path) = args.file {
        app.new_viewer(path);
    }

    app.run_app(&mut terminal)
}
