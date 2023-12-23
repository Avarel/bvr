use std::sync::Arc;

use regex::bytes::Regex;

use crate::buf::ContiguousSegmentIterator;
use crate::inflight_tool::{Inflight, InflightImpl, Inflightable};
use crate::SegBuffer;
use crate::{index::BufferIndex, Result};

use super::{BufferMatches, Matches, IncompleteMatches};

impl Inflightable for Matches {
    type Incomplete = IncompleteMatches;

    type Remote = InflightSearchRemote;

    fn make_remote(inner: Arc<crate::inflight_tool::InflightImpl<Self>>) -> Self::Remote {
        InflightSearchRemote(inner)
    }

    fn finish(inner: Self::Incomplete) -> Self {
        inner.finish()
    }

    fn snapshot(inner: &Self::Incomplete) -> Self {
        inner.inner.clone()
    }
}

impl InflightImpl<Matches> {
    fn search<Idx>(
        self: Arc<Self>,
        mut iter: ContiguousSegmentIterator<Idx>,
        regex: Regex,
    ) -> Result<()>
    where
        Idx: BufferIndex,
    {
        while let Some((idx, start, buf)) = iter.next_buf() {
            if Arc::strong_count(&self) <= 1 {
                break;
            }

            self.write(|inner| {
                for res in regex.find_iter(buf) {
                    let match_start = res.start() as u64 + start;
                    inner.add_line(idx.line_of_data(match_start).unwrap())
                }
            });
        }
        self.mark_complete();
        Ok(())
    }
}

pub struct InflightSearchRemote(Arc<InflightImpl<Matches>>);

impl InflightSearchRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn search<Idx>(self, iter: ContiguousSegmentIterator<Idx>, regex: Regex) -> Result<()>
    where
        Idx: BufferIndex,
    {
        self.0.search(iter, regex)
    }
}

impl Inflight<Matches> {
    /// Searches for a regular expression pattern in a segmented buffer.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `InflightSearch` object
    /// if the internal iterator creation was successful, and an error otherwise.
    pub fn search<Idx>(buf: &SegBuffer<Idx>, regex: Regex) -> Result<Self>
    where
        Idx: BufferIndex + Clone + Send + 'static,
    {
        let (search, remote) = Inflight::<Matches>::new();
        std::thread::spawn({
            let iter = buf.segment_iter()?;
            move || remote.search(iter, regex)
        });
        Ok(search)
    }
}

impl BufferMatches for Inflight<Matches> {
    fn is_complete(&self) -> bool {
        self.is_complete()
    }

    fn get(&self, idx: usize) -> Option<usize> {
        match self {
            Self::Incomplete(inner) => inner.read(|index| index.get(idx)),
            Self::Complete(index) => index.get(idx),
        }
    }

    fn has_line(&self, line_number: usize) -> bool {
        match self {
            Self::Incomplete(inner) => inner.read(|index| index.has_line(line_number)),
            Self::Complete(index) => index.has_line(line_number),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Incomplete(inner) => inner.read(|index| index.len()),
            Self::Complete(index) => index.len(),
        }
    }
}

pub type InflightSearch = Inflight<Matches>;
