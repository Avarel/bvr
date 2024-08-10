use super::{super::filters::FilterExportSet, storage_dir_create, APP_ID, FILTER_FILE};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{cell::OnceCell, path::PathBuf};

pub struct FilterData {
    path: Option<PathBuf>,
    state: OnceCell<LoadedFilterData>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct LoadedFilterData {
    persistent: bool,
    filters: Vec<FilterExportSet>,
}

impl FilterData {
    pub fn new() -> Self {
        Self {
            path: storage_dir_create(APP_ID)
                .map(|path| path.join(FILTER_FILE))
                .ok(),
            state: OnceCell::new(),
        }
    }

    fn init(&self) {
        self.state.get_or_init(|| {
            self.path
                .as_ref()
                .and_then(|path| std::fs::File::open(path).ok())
                .map(std::io::BufReader::new)
                .and_then(|reader| serde_json::from_reader::<_, LoadedFilterData>(reader).ok())
                .unwrap_or_else(LoadedFilterData::default)
        });
    }

    fn load_and_save<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut LoadedFilterData),
    {
        self.init();
        let data = self.state.get_mut().unwrap();

        f(data);

        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer(writer, data)?;
        Ok(())
    }

    fn read<'a, F, R>(&'a self, f: F) -> Result<R>
    where
        F: FnOnce(&'a LoadedFilterData) -> R,
    {
        self.init();
        let data = self.state.get().unwrap();
        Ok(f(data))
    }

    pub fn set_persistent(&mut self, persistent: bool) -> Result<()> {
        self.load_and_save(|data| {
            data.persistent = persistent;
        })
    }

    pub fn is_persistent(&self) -> Result<bool> {
        self.read(|data| data.persistent)
    }

    pub fn filters(&self) -> Result<&[FilterExportSet]> {
        self.read(|data| data.filters.as_ref())
    }

    pub fn add_filter(&mut self, filter: FilterExportSet) -> Result<()> {
        self.load_and_save(|data| {
            data.filters.clear(); // TODO: Get rid of this once UI is finalized
            data.filters.push(filter);
        })
    }

    pub fn remove_filter(&mut self, index: usize) -> Result<()> {
        self.load_and_save(|data| {
            data.filters.remove(index);
        })
    }
}
