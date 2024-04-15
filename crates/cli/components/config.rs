use super::filters::FilterExportSet;
use crate::components::filters::FilterExport;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_ID: &str = "bvr";

pub const CONFIG_FILE: &str = "config.toml";
pub const FILTER_SAVE_FILE: &str = "filters.json";

pub enum FilterSaveData {
    Unloaded,
    Loaded(LoadedFilterSaveData),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoadedFilterSaveData {
    pub filters: Vec<FilterExportSet>,
}

pub fn storage_dir(app_id: &str) -> Option<PathBuf> {
    directories_next::ProjectDirs::from("", "", app_id)
        .map(|proj_dirs| proj_dirs.data_dir().to_path_buf())
}

pub fn storage_dir_create(app_id: &str) -> Result<PathBuf> {
    let path = storage_dir(app_id).ok_or(bvr_core::err::Error::Internal)?;
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

impl FilterSaveData {
    pub fn new() -> Self {
        Self::Unloaded
    }

    pub fn load(&mut self) -> Result<&mut LoadedFilterSaveData> {
        let path = storage_dir_create(APP_ID)?.join(FILTER_SAVE_FILE);

        match std::fs::File::open(path) {
            Ok(file) => {
                let reader = std::io::BufReader::new(file);
                match serde_json::from_reader::<_, LoadedFilterSaveData>(reader) {
                    Ok(data) => *self = Self::Loaded(data),
                    Err(_) => {
                        *self = Self::Loaded(LoadedFilterSaveData {
                            filters: Vec::new(),
                        })
                    }
                }
            }
            Err(_) => {
                *self = Self::Loaded(LoadedFilterSaveData {
                    filters: Vec::new(),
                })
            }
        }
        match self {
            FilterSaveData::Loaded(data) => Ok(data),
            FilterSaveData::Unloaded => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn save(&self) -> Result<()> {
        match self {
            FilterSaveData::Unloaded => Ok(()),
            FilterSaveData::Loaded(data) => {
                let path = storage_dir_create(APP_ID)?.join(FILTER_SAVE_FILE);
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)?;
                let writer = std::io::BufWriter::new(file);
                serde_json::to_writer(writer, data)?;
                Ok(())
            }
        }
    }

    pub fn filters(&mut self) -> Result<&[FilterExportSet]> {
        Ok(self.load()?.filters.as_slice())
    }

    pub fn add_filter(&mut self, filter: FilterExportSet) -> Result<()> {
        let filters = &mut self.load()?.filters;
        filters.clear(); // TODO: Get rid of this once UI is finalized
        filters.push(filter);
        Ok(())
    }

    pub fn remove_filter(&mut self, index: usize) -> Result<()> {
        self.load()?.filters.remove(index);
        Ok(())
    }
}
