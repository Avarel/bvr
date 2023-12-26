use crate::{
    buf::segment::{Segment, SegmentMut},
    cowvec::inflight::{InflightVec, InflightVecWriter},
    err::{Error, Result},
};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::{fs::File, sync::Arc};

struct IndexingTask {
    /// This is the sender side of the channel that receives byte indexes of `\n`.
    sx: Sender<u64>,
    segment: Segment,
}

impl IndexingTask {
    #[inline]
    fn new(file: &File, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let segment = Segment::map_file(start..end, file)?;
        let (sx, rx) = std::sync::mpsc::channel();
        Ok((Self { sx, segment }, rx))
    }

    fn compute(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.segment) {
            self.sx
                .send(self.segment.start() + i as u64 + 1)
                .map_err(|_| Error::Internal)?;
        }

        Ok(())
    }
}

/// Generalized type for streams passed into [InflightIndexRemote].
pub type BoxedStream = Box<dyn std::io::Read + Send>;

/// A remote type that can be used to set off the indexing process of a
/// file or a stream.
pub struct LineIndexRemote(Arc<InflightVecWriter<u64>>);

impl LineIndexRemote {
    pub fn index_file(self, file: File) -> Result<()> {
        // Build index
        let (sx, rx) = std::sync::mpsc::sync_channel(4);

        let len = file.metadata()?.len();
        let file = file.try_clone()?;

        self.0.write(|inner| inner.push(0));

        // We have up to 3 primary holders:
        // - Someone who needs to read the inflight
        // - The mapping worker
        // - The indexing worker
        // While the indexing worker is alive, there will be at least a mapping
        // and indexing worker. So the inflight only needs to run while there is
        // someone who needs to read the inflight, and thus the strong count
        // must be at least 3 for this to be useful

        // Indexing worker
        let spawner: JoinHandle<Result<()>> = std::thread::spawn({
            let r = self.0.clone();
            move || {
                let mut curr = 0;

                while curr < len && Arc::strong_count(&r) >= 3 {
                    let end = (curr + Segment::MAX_SIZE).min(len);
                    let (task, task_rx) = IndexingTask::new(&file, curr, end)?;
                    sx.send(task_rx).unwrap();

                    std::thread::spawn(|| task.compute());

                    curr = end;
                }

                Ok(())
            }
        });

        while let Ok(task_rx) = rx.recv() {
            while let Ok(line_data) = task_rx.recv() {
                self.0.write(|inner| inner.push(line_data));
            }
        }

        spawner.join().unwrap().unwrap();
        self.0.write(|inner| inner.push(len));
        Ok(())
    }

    pub fn index_stream(self, mut stream: BoxedStream, outgoing: Sender<Segment>) -> Result<()> {
        let mut len = 0;

        self.0.write(|inner| inner.push(0));

        loop {
            let mut segment = SegmentMut::new(len)?;

            let mut buf_len = 0;
            loop {
                match stream.read(&mut segment[buf_len..])? {
                    0 => break,
                    l => buf_len += l,
                }
            }

            self.0.write(|inner| {
                for i in memchr::memchr_iter(b'\n', &segment) {
                    let line_data = len + i as u64;
                    inner.push(line_data + 1);
                }
            });

            outgoing
                .send(segment.into_read_only()?)
                .map_err(|_| Error::Internal)?;

            if buf_len == 0 {
                break;
            }

            len += buf_len as u64;
        }

        self.0.write(|inner| inner.push(len));
        self.0.mark_complete();
        Ok(())
    }
}

#[derive(Clone)]
pub struct LineIndex(InflightVec<u64>);

impl LineIndex {
    #[inline]
    pub fn new() -> (Self, LineIndexRemote) {
        let inner = Arc::new(InflightVecWriter::<u64>::new());
        (
            Self(InflightVec::Incomplete(inner.clone())),
            LineIndexRemote(inner),
        )
    }

    pub fn new_complete(file: File) -> Result<Self> {
        let (index, remote) = Self::new();
        remote.index_file(file)?;
        Ok(index)
    }

    pub fn line_count(&self) -> usize {
        self.0.read(|index| index.len().saturating_sub(1))
    }

    pub fn data_of_line(&self, line_number: usize) -> Option<u64> {
        self.0.read(|index| index.get(line_number))
    }

    pub fn line_of_data(&self, key: u64) -> Option<usize> {
        self.0.read(|index| {
            // Safety: this code was pulled from Vec::binary_search_by
            let mut size = index.len() - 1;
            let mut left = 0;
            let mut right = size;
            while left < right {
                let mid = left + size / 2;

                // mid must be less than size, which is self.line_index.len() - 1
                let start = unsafe { *index.get_unchecked(mid) };
                let end = unsafe { *index.get_unchecked(mid + 1) };

                if end <= key {
                    left = mid + 1;
                } else if start > key {
                    right = mid;
                } else {
                    return Some(mid);
                }

                size = right - left;
            }

            None
        })
    }

    #[inline]
    pub fn try_finalize(&mut self) -> bool {
        self.0.try_finalize()
    }
}
