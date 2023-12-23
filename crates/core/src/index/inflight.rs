//! Contains the [InflightIndex] and [InflightIndexRemote], which are abstractions
//! that allow the use of [IncompleteIndex] functionalities while it is "inflight"
//! or in the middle of the indexing operation.

use super::{BufferIndex, IncompleteIndex, Index};
use crate::{
    buf::segment::{Segment, SegmentMut},
    err::{Error, Result},
    inflight_tool::{Inflight, InflightImpl, Inflightable},
};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::{fs::File, sync::Arc};

/// Internal indexing task used by [InflightIndexImpl].
struct IndexingTask {
    /// This is the sender side of the channel that receives byte indexes of `\n`.
    sx: Sender<u64>,
    segment: Segment,
}

impl IndexingTask {
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

impl Inflightable for Index {
    type Incomplete = IncompleteIndex;

    type Remote = InflightIndexRemote;

    fn make_remote(inner: Arc<crate::inflight_tool::InflightImpl<Self>>) -> Self::Remote {
        InflightIndexRemote(inner)
    }

    fn finish(inner: Self::Incomplete) -> Self {
        inner.finish()
    }

    fn snapshot(inner: &Self::Incomplete) -> Self {
        inner.inner.clone()
    }
}

/// Generalized type for streams passed into [InflightIndexRemote].
pub type Stream = Box<dyn std::io::Read + Send>;

impl InflightImpl<Index> {
    fn index_file(self: Arc<Self>, file: File) -> Result<()> {
        // Build index
        let (sx, rx) = std::sync::mpsc::sync_channel(4);

        let len = file.metadata()?.len();
        let file = file.try_clone()?;

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
            let r = self.clone();
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
                self.write(|inner| inner.push_line_data(line_data));
            }
        }

        spawner.join().unwrap().unwrap();
        self.write(|inner| inner.finalize(len));
        Ok(())
    }

    fn index_stream(self: Arc<Self>, mut stream: Stream, outgoing: Sender<Segment>) -> Result<()> {
        let mut len = 0;

        loop {
            let mut segment = SegmentMut::new(len)?;

            let mut buf_len = 0;
            loop {
                match stream.read(&mut segment[buf_len..])? {
                    0 => break,
                    l => buf_len += l,
                }
            }

            self.write(|inner| {
                for i in memchr::memchr_iter(b'\n', &segment) {
                    let line_data = len + i as u64;
                    inner.push_line_data(line_data + 1);
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

        self.write(|inner| inner.finalize(len));
        self.mark_complete();
        Ok(())
    }
}

/// A remote type that can be used to set off the indexing process of a
/// file or a stream.
pub struct InflightIndexRemote(Arc<InflightImpl<Index>>);

impl InflightIndexRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn index_file(self, file: File) -> Result<()> {
        self.0.index_file(file)
    }

    /// Index a stream and load the data into the associated [InflightIndex].
    pub fn index_stream(self, stream: Stream, outgoing: Sender<Segment>) -> Result<()> {
        self.0.index_stream(stream, outgoing)
    }
}

impl Inflight<Index> {
    /// Creates a new complete index from a file.
    ///
    /// This function creates a new complete index from the provided file. It internally
    /// initializes an [InflightIndex] in file mode, indexes the file, and returns the
    /// resulting [CompleteIndex].
    ///
    /// # Returns
    ///
    /// A `Result` containing the `CompleteIndex` if successful, or an error if the
    /// indexing process fails.
    pub fn new_complete(file: &File) -> Result<Index> {
        let (result, indexer) = Self::new();
        indexer.index_file(file.try_clone()?)?;
        Ok(result.unwrap())
    }
}
impl BufferIndex for Inflight<Index> {
    fn line_count(&self) -> usize {
        match self {
            Self::Incomplete(inner) => inner.read(|index| (index.line_count())),
            Self::Complete(index) => index.line_count(),
        }
    }

    fn data_of_line(&self, line_number: usize) -> Option<u64> {
        match self {
            Self::Incomplete(inner) => inner.read(|index| (index.data_of_line(line_number))),
            Self::Complete(index) => index.data_of_line(line_number),
        }
    }

    fn line_of_data(&self, start: u64) -> Option<usize> {
        match self {
            Self::Incomplete(inner) => inner.read(|index| (index.line_of_data(start))),
            Self::Complete(index) => index.line_of_data(start),
        }
    }
}

pub type InflightIndex = Inflight<Index>;
