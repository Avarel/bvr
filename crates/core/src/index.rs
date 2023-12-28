use crate::{
    buf::segment::{Segment, SegmentMut},
    collections::cowvec::{CowVec, CowVecWriter},
    err::{Error, Result},
};
use std::fs::File;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

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

/// Generalized type for streams passed into [LineIndex].
pub type BoxedStream = Box<dyn std::io::Read + Send>;

/// A remote type that can be used to set off the indexing process of a
/// file or a stream.
struct LineIndexRemote {
    buf: CowVecWriter<u64>,
}

impl LineIndexRemote {
    const BYTES_PER_LINE_HEURISTIC: u64 = 128;

    pub fn index_file(mut self, file: File) -> Result<()> {
        // Build index
        let (sx, rx) = std::sync::mpsc::sync_channel(4);

        let len = file.metadata()?.len();
        let file = file.try_clone()?;

        self.buf
            .reserve((len / Self::BYTES_PER_LINE_HEURISTIC) as usize);
        self.buf.push(0);

        // Indexing worker
        let spawner: JoinHandle<Result<()>> = std::thread::spawn(move || {
            let mut curr = 0;

            while curr < len {
                let end = (curr + Segment::MAX_SIZE).min(len);
                let (task, task_rx) = IndexingTask::new(&file, curr, end)?;
                sx.send(task_rx).map_err(|_| Error::Internal)?;

                std::thread::spawn(|| task.compute());

                curr = end;
            }

            Ok(())
        });

        while let Ok(task_rx) = rx.recv() {
            if !self.buf.has_readers() {
                break;
            }

            while let Ok(line_data) = task_rx.recv() {
                self.buf.push(line_data);
            }
        }

        spawner.join().map_err(|_| Error::Internal)??;
        self.buf.push(len);

        Ok(())
    }

    pub fn index_stream(
        mut self,
        mut stream: BoxedStream,
        outgoing: Sender<Segment>,
    ) -> Result<()> {
        let mut len = 0;

        self.buf.push(0);

        loop {
            let mut segment = SegmentMut::new(len)?;

            let mut buf_len = 0;
            loop {
                match stream.read(&mut segment[buf_len..])? {
                    0 => break,
                    l => buf_len += l,
                }
            }

            for i in memchr::memchr_iter(b'\n', &segment) {
                let line_data = len + i as u64;
                self.buf.push(line_data + 1);
            }

            outgoing
                .send(segment.into_read_only()?)
                .map_err(|_| Error::Internal)?;

            if buf_len == 0 {
                break;
            }

            len += buf_len as u64;
        }

        self.buf.push(len);
        Ok(())
    }
}

#[derive(Clone)]
pub struct LineIndex {
    buf: CowVec<u64>,
}

impl LineIndex {
    #[inline]
    pub fn read_file(file: File, complete: bool) -> Result<Self> {
        let (buf, writer) = CowVec::new();
        let task = move || LineIndexRemote { buf: writer }.index_file(file);
        if complete {
            task()?;
        } else {
            std::thread::spawn(task);
        }
        Ok(Self { buf })
    }

    #[inline]
    pub fn read_stream(
        stream: BoxedStream,
        outgoing: Sender<Segment>,
        complete: bool,
    ) -> Result<Self> {
        let (buf, writer) = CowVec::new();
        let task = move || LineIndexRemote { buf: writer }.index_stream(stream, outgoing);
        if complete {
            task()?;
        } else {
            std::thread::spawn(task);
        }
        Ok(Self { buf })
    }

    pub fn line_count(&self) -> usize {
        self.buf.len().saturating_sub(1)
    }

    pub fn data_of_line(&self, line_number: usize) -> Option<u64> {
        self.buf.get(line_number)
    }

    pub fn line_of_data(&self, key: u64) -> Option<usize> {
        // Safety: this code was pulled from Vec::binary_search_by
        let buf = self.buf.snapshot();
        let mut size = buf.len().saturating_sub(1);
        let mut left = 0;
        let mut right = size;
        while left < right {
            let mid = left + size / 2;

            // mid must be less than size, which is self.line_index.len() - 1
            let start = unsafe { buf.get_unchecked(mid) };
            let end = unsafe { buf.get_unchecked(mid + 1) };

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
    }
}
