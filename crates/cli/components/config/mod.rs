pub mod filter;

use anyhow::Result;
use std::path::PathBuf;

const APP_ID: &str = "bvr";

#[allow(dead_code)]
const CONFIG_FILE: &str = "config.toml";
const FILTER_FILE: &str = "filters.json";

fn storage_dir(app_id: &str) -> Option<PathBuf> {
    directories_next::ProjectDirs::from("", "", app_id)
        .map(|proj_dirs| proj_dirs.data_dir().to_path_buf())
}

fn storage_dir_create(app_id: &str) -> Result<PathBuf> {
    let path = storage_dir(app_id).ok_or(bvr_core::err::Error::Internal)?;
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
