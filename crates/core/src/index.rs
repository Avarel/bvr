use std::{
    fs::File,
    sync::{
        atomic::AtomicU32,
        mpsc::{Receiver, Sender},
        Arc,
    },
    thread::JoinHandle,
};

use crate::{
    buf::segment::{Segment, SegmentMut},
    cowvec::{CowVec, CowVecWriter},
    err::{Error, Result},
};

struct IndexingTask {
    /// This is the sender side of the channel that receives byte indexes of `\n`.
    sx: Sender<(usize, Vec<IndexType>)>,
    segment: Segment,
}

impl IndexingTask {
    #[inline]
    fn new(file: &File, start: u64, end: u64) -> Result<(Self, Receiver<(usize, Vec<IndexType>)>)> {
        let segment = Segment::map_file(start..end, file)?;
        let (sx, rx) = std::sync::mpsc::channel();
        Ok((Self { sx, segment }, rx))
    }

    fn compute(self) -> Result<()> {
        let mut curr_upper = 0;
        let mut lowers = Vec::new();
        for i in memchr::memchr_iter(b'\n', &self.segment) {
            let line_data = self.segment.start() + i as u64;
            let upper = (line_data >> IndexType::BITS) as usize;
            let lower = line_data as IndexType;

            if upper > curr_upper {
                if !lowers.is_empty() {
                    self.sx
                        .send((curr_upper, std::mem::take(&mut lowers)))
                        .map_err(|_| Error::Internal)?;
                }

                curr_upper = upper;
            }

            lowers.push(lower);
        }

        if !lowers.is_empty() {
            self.sx
                .send((curr_upper, lowers))
                .map_err(|_| Error::Internal)?;
        }

        Ok(())
    }
}

/// Generalized type for streams passed into [LineIndex].
pub type BoxedStream = Box<dyn std::io::Read + Send>;

pub struct ProgressReport {
    progress: Option<AtomicU32>,
}

impl ProgressReport {
    pub const PERCENT: Self = Self {
        progress: Some(AtomicU32::new(0f32.to_bits())),
    };

    pub const NONE: Self = Self { progress: None };

    pub fn progress(&self) -> Option<f32> {
        self.progress
            .as_ref()
            .map(|v| v.load(std::sync::atomic::Ordering::Relaxed))
            .map(f32::from_bits)
    }

    fn store_progress(&self, val: f32) {
        if let Some(progress) = self.progress.as_ref() {
            progress.store(val.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn complete(&self) {
        if let Some(progress) = self.progress.as_ref() {
            progress.store(1f32.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
    }
}

// Debug builds use a smaller index type to make it easier to catch issues.
#[cfg(debug_assertions)]
type IndexType = u8;

#[cfg(not(debug_assertions))]
type IndexType = u32;

/// A remote type that can be used to set off the indexing process of a
/// file or a stream.
pub(crate) struct LineIndexWriter {
    upper: CowVecWriter<(usize, usize)>,
    lower: CowVecWriter<IndexType>,
    report: Arc<ProgressReport>,
    curr_upper: usize,
}

impl LineIndexWriter {
    const BYTES_PER_LINE_HEURISTIC: u64 = 128;

    pub fn index_file(mut self, file: File) -> Result<()> {
        // Build index
        let (sx, rx) = std::sync::mpsc::sync_channel(4);

        let len = file.metadata()?.len();
        let file = file.try_clone()?;

        self.lower
            .reserve((len / Self::BYTES_PER_LINE_HEURISTIC) as usize);
        self.lower.push(0);

        // Indexing worker
        let spawner: JoinHandle<Result<()>> = std::thread::spawn({
            let report = self.report.clone();
            move || {
                let mut curr = 0;

                while curr < len {
                    let end = (curr + SegmentMut::TODO_REMOVE_SIZE).min(len);
                    let (task, task_rx) = IndexingTask::new(&file, curr, end)?;
                    sx.send(task_rx).map_err(|_| Error::Internal)?;

                    std::thread::spawn(|| task.compute());

                    curr = end;

                    report.store_progress(curr as f32 / len as f32);
                }

                Ok(())
            }
        });

        while let Ok(task_rx) = rx.recv() {
            if !self.lower.has_readers() {
                break;
            }

            while let Ok((upper, lowers)) = task_rx.recv() {
                if upper > self.curr_upper {
                    self.upper.push((self.lower.len(), upper));
                    self.curr_upper = upper;
                }
                self.lower.extend_from_slice(&lowers);
            }
        }

        spawner.join().map_err(|_| Error::Internal)??;
        self.push(len);

        Ok(())
    }

    pub fn push(&mut self, line_data: u64) {
        let upper = (line_data >> IndexType::BITS) as usize;
        let lower = line_data as IndexType;

        if upper > self.curr_upper {
            self.upper.push((self.lower.len(), upper));
            self.curr_upper = upper;
        }

        self.lower.push(lower);
    }

    pub fn index_stream(
        mut self,
        mut stream: BoxedStream,
        outgoing: Sender<Segment>,
        segment_size: u64,
    ) -> Result<()> {
        let mut len = 0;

        self.lower.push(0);

        loop {
            let mut segment = SegmentMut::new(len, segment_size)?;

            let mut buf_len = 0;
            loop {
                match stream.read(&mut segment[buf_len..])? {
                    0 => break,
                    l => buf_len += l,
                }
            }

            for i in memchr::memchr_iter(b'\n', &segment) {
                let line_data = len + i as u64;
                self.push(line_data + 1);
            }

            outgoing
                .send(segment.into_read_only()?)
                .map_err(|_| Error::Internal)?;

            if buf_len == 0 {
                break;
            }

            len += buf_len as u64;
        }

        self.push(len);
        Ok(())
    }
}

impl Drop for LineIndexWriter {
    fn drop(&mut self) {
        self.report.complete();
    }
}

#[derive(Clone)]
pub struct LineIndex {
    // This stores the indices of buf where the first index represents an index in buf
    // that overflows, and the second index represents how many times it overflows.
    // For example, if overflow[0] = (1000, 2), then buf[1000] represents a number
    // that is 2 * IndexType::MAX larger than the value stored in buf[1000].
    //
    // This allows us to compress the line index by storing only the lower bits of the
    // index in buf, and storing the upper bits in overflow only when necessary.
    upper: Arc<CowVec<(usize, usize)>>,
    lower: Arc<CowVec<IndexType>>,
    report: Arc<ProgressReport>,
}

impl LineIndex {
    pub(crate) fn new(report: ProgressReport) -> (Self, LineIndexWriter) {
        let (upper, writer_overflow) = CowVec::new();
        let (lower, writer) = CowVec::new();
        let report = Arc::new(report);
        let writer = {
            let report = report.clone();
            LineIndexWriter {
                lower: writer,
                upper: writer_overflow,
                report,
                curr_upper: 0,
            }
        };
        (
            Self {
                lower,
                upper,
                report,
            },
            writer,
        )
    }

    pub fn read_file(file: File, complete: bool) -> Result<Self> {
        let (index, writer) = Self::new(ProgressReport::PERCENT);
        let task = move || writer.index_file(file);
        if complete {
            task()?;
        } else {
            std::thread::spawn(task);
        }
        Ok(index)
    }

    pub fn read_stream(
        stream: BoxedStream,
        outgoing: Sender<Segment>,
        block_until_complete: bool,
        segment_size: u64,
    ) -> Result<Self> {
        let (index, writer) = Self::new(ProgressReport::NONE);
        let task = move || writer.index_stream(stream, outgoing, segment_size);
        if block_until_complete {
            task()?;
        } else {
            std::thread::spawn(task);
        }
        Ok(index)
    }

    pub fn report(&self) -> &ProgressReport {
        &self.report
    }

    pub fn line_count(&self) -> usize {
        self.lower.len().saturating_sub(1)
    }

    pub fn upper_bits(&self, line_number: usize) -> u64 {
        // Find first entry where key >= index
        let upper_bits = 'binary_search: {
            let buf = self.upper.snapshot();

            let mut size = buf.len();
            if size == 0 {
                break 'binary_search 0;
            }
            let mut base = 0usize;

            // Based on std::slice::binary_search_by, specialized for the container
            while size > 1 {
                let half = size / 2;
                let mid = base + half;
                let &(i, _) = unsafe { buf.get_unchecked(mid) };
                base = std::hint::select_unpredictable(i > line_number, base, mid);
                size -= half;
            }

            let &(i, diff) = unsafe { buf.get_unchecked(base) };
            if i <= line_number {
                diff as u64
            } else {
                0
            }
        };

        upper_bits << IndexType::BITS as u64
    }

    pub fn data_of_line(&self, line_number: usize) -> Option<u64> {
        // Get the lower bits from buf and add the upper bits from overflow.
        self.lower
            .get(line_number)
            .map(|lower_bits| lower_bits as u64 + self.upper_bits(line_number))
    }

    pub fn line_of_data(&self, key: u64) -> Option<usize> {
        let buf = self.lower.snapshot();
        let mut size = buf.len().saturating_sub(1);
        if size == 0 {
            return None;
        }

        // Based on std::slice::binary_search_by, specialized for the container
        // Find last line where data_of_line(line) <= key
        let mut base = 0;
        while size > 1 {
            let half = size / 2;
            let mid = base + half;
            let start = unsafe { self.data_of_line(mid).unwrap_unchecked() };
            base = std::hint::select_unpredictable(start > key, base, mid);
            size -= half;
        }

        // Verify the candidate is a valid match
        let start = unsafe { self.data_of_line(base).unwrap_unchecked() };
        let end = unsafe { self.data_of_line(base + 1).unwrap_unchecked() };
        (start <= key && key < end).then_some(base)
    }

    pub fn is_complete(&self) -> bool {
        self.lower.is_complete()
    }
}
