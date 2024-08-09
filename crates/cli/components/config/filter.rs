use super::{super::filters::FilterExportSet, storage_dir_create, APP_ID, FILTER_FILE};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

pub struct FilterData {
    state: RefCell<FilterDataState>,
}

enum FilterDataState {
    Unloaded,
    Loaded(LoadedFilterData),
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct LoadedFilterData {
    persistent: bool,
    filters: Vec<FilterExportSet>,
}

impl FilterData {
    pub fn new() -> Self {
        Self {
            state: RefCell::new(FilterDataState::Unloaded),
        }
    }

    fn is_loaded(&self) -> bool {
        matches!(&*self.state.borrow(), FilterDataState::Loaded(_))
    }

    fn load_and_save<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut LoadedFilterData),
    {
        let path = storage_dir_create(APP_ID)?.join(FILTER_FILE);

        let mut state = self.state.borrow_mut();
        *state = FilterDataState::Loaded(
            std::fs::File::open(&path)
                .ok()
                .map(std::io::BufReader::new)
                .and_then(|reader| serde_json::from_reader::<_, LoadedFilterData>(reader).ok())
                .unwrap_or_else(LoadedFilterData::default),
        );
        match &mut *state {
            FilterDataState::Loaded(data) => {
                f(data);

                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)?;
                let writer = std::io::BufWriter::new(file);
                serde_json::to_writer(writer, data)?;
            }
            FilterDataState::Unloaded => unsafe { std::hint::unreachable_unchecked() },
        }
        Ok(())
    }

    fn read<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&LoadedFilterData) -> R,
    {
        if !self.is_loaded() {
            let path = storage_dir_create(APP_ID)?.join(FILTER_FILE);
            let mut state = self.state.borrow_mut();
            *state = std::fs::File::open(path)
                .ok()
                .map(std::io::BufReader::new)
                .and_then(|reader| serde_json::from_reader::<_, LoadedFilterData>(reader).ok())
                .map(FilterDataState::Loaded)
                .unwrap_or(FilterDataState::Unloaded);
        }

        Ok(match &*self.state.borrow() {
            FilterDataState::Unloaded => f(&LoadedFilterData::default()),
            FilterDataState::Loaded(data) => f(data),
        })
    }

    pub fn set_persistent(&mut self, persistent: bool) -> Result<()> {
        self.load_and_save(|data| {
            data.persistent = persistent;
        })
    }

    pub fn is_persistent(&self) -> Result<bool> {
        self.read(|data| data.persistent)
    }

    pub fn filters(&self) -> Result<Vec<FilterExportSet>> {
        self.read(|data| data.filters.clone())
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
