mod app;
mod colors;
mod components;
mod direction;

use anyhow::Result;
use app::App;
use clap::Parser;
use ratatui::{prelude::CrosstermBackend, Terminal};
use std::{io::IsTerminal, path::PathBuf};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Files to open in the pager
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let stdout = std::io::stdout().lock();
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    let mut app = App::new(terminal);

    for path in args.files {
        app.open_file(&path)?;
    }

    if !std::io::stdin().is_terminal() {
        app.open_stream(String::from("Pipe Stream"), Box::new(std::io::stdin()))?;
    }

    app.run_app()
}

fn regex_compile(pattern: &str) -> std::result::Result<regex::bytes::Regex, regex::Error> {
    regex::bytes::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
}
