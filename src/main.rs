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