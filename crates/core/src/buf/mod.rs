//! The `buf` module contains the [SegBuffer] struct, which is the main
//! interface for creating and interacting with the segmented buffers.

pub mod segment;

use self::segment::{SegBytes, SegStr, Segment};
use crate::{index::BoxedStream, LineIndex, LineSet, Result};
use lru::LruCache;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Seek, Write};
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

/// A segmented buffer that holds data in multiple segments.
///
/// The `Buffer` struct represents a buffer that is divided into multiple segments.
/// It contains the [LineIndex] and the internal representation of the segments.
pub struct SegBuffer {
    /// The [LineIndex] of this buffer.
    index: LineIndex,
    /// The internal representation of this buffer.
    map: BufferMap,
}

struct BufferMap {
    repr: BufferRepr,
    segment_size: u64,
}

struct StreamInner {
    pending_segs: Option<Receiver<Segment>>,
    segments: Vec<Arc<Segment>>,
}

/// Internal representation of the segmented buffer, which allows for working
/// with both files and streams of data. All segments are assumed to have
/// the same size with the exception of the last segment.
enum BufferRepr {
    /// Data can be loaded on demand.
    File {
        file: File,
        len: u64,
        segments: RefCell<LruCache<usize, Arc<Segment>>>,
    },
    /// Data is all present in memory in multiple anonymous mmaps.
    Stream(RefCell<StreamInner>),
}

impl BufferMap {
    #[inline]
    pub fn id_of_data(&self, start: u64) -> usize {
        (start / self.segment_size) as usize
    }

    #[inline]
    pub fn data_range_of_id(&self, id: usize) -> Range<u64> {
        let start = id as u64 * self.segment_size;
        start..start + self.segment_size
    }

    fn fetch(&self, seg_id: usize) -> Option<Arc<Segment>> {
        match &self.repr {
            BufferRepr::File {
                file,
                len,
                segments,
            } => {
                let range = self.data_range_of_id(seg_id);
                let range = range.start..range.end.min(*len);
                Some(
                    segments
                        .borrow_mut()
                        .get_or_insert(seg_id, || {
                            Arc::new(Segment::map_file(range, file).expect("mmap was successful"))
                        })
                        .clone(),
                )
            }
            BufferRepr::Stream(inner) => {
                let StreamInner {
                    pending_segs,
                    segments,
                } = &mut *inner.borrow_mut();
                if let Some(rx) = pending_segs {
                    loop {
                        match rx.try_recv() {
                            Ok(segment) => {
                                #[cfg(debug_assertions)]
                                if let Some(first_segment) = segments.first() {
                                    debug_assert_eq!(first_segment.len(), segment.len())
                                }
                                segments.push(Arc::new(segment))
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                *pending_segs = None;
                                break;
                            }
                        }
                    }
                }
                segments.get(seg_id).cloned()
            }
        }
    }
}

impl SegBuffer {
    const SEGMENT_SIZE: u64 = 1 << 20;

    pub fn read_file(file: File, seg_count: NonZeroUsize, complete: bool) -> Result<Self> {
        file.try_lock_shared()?;
        let index = LineIndex::read_file(file.try_clone()?, complete)?;

        Ok(Self {
            index,
            map: BufferMap {
                repr: BufferRepr::File {
                    len: file.metadata()?.len(),
                    file,
                    segments: RefCell::new(LruCache::new(seg_count)),
                },
                segment_size: Self::SEGMENT_SIZE,
            },
        })
    }

    pub fn read_stream(stream: BoxedStream, complete: bool) -> Result<Self> {
        let (sx, rx) = std::sync::mpsc::channel();
        let index = LineIndex::read_stream(stream, sx, complete, Self::SEGMENT_SIZE)?;

        Ok(Self {
            index,
            map: BufferMap {
                repr: BufferRepr::Stream(RefCell::new(StreamInner {
                    pending_segs: Some(rx),
                    segments: Vec::new(),
                })),
                segment_size: Self::SEGMENT_SIZE,
            },
        })
    }

    /// Return the line count of this [SegBuffer].
    #[inline]
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    /// Return the [LineIndex] of this [SegBuffer].
    #[inline]
    pub fn index(&self) -> &LineIndex {
        &self.index
    }

    pub fn get_bytes(&self, line_number: usize) -> Option<SegBytes> {
        assert!(line_number <= self.line_count());

        let data_start = self.index.data_of_line(line_number)?;
        let data_end = self.index.data_of_line(line_number + 1)?;
        let seg_start = self.map.id_of_data(data_start);
        let seg_end = self.map.id_of_data(data_end);

        if seg_start == seg_end {
            // The data is in a single segment
            let seg = self.map.fetch(seg_start)?;
            let range = seg.translate_inner_data_range(data_start, data_end);
            Some(seg.get_bytes(range))
        } else {
            debug_assert!(seg_start < seg_end);
            // The data may cross several segments, so we must piece together
            // the data from across the segments.
            let mut buf = Vec::with_capacity((data_end - data_start) as usize);

            let seg_first = self.map.fetch(seg_start)?;
            let seg_last = self.map.fetch(seg_end)?;
            let (start, end) = (
                seg_first.translate_inner_data_index(data_start),
                seg_last.translate_inner_data_index(data_end),
            );
            buf.extend_from_slice(&seg_first[start as usize..]);
            for seg_id in seg_start + 1..seg_end {
                buf.extend_from_slice(&self.map.fetch(seg_id)?);
            }
            buf.extend_from_slice(&seg_last[..end as usize]);

            Some(SegBytes::new_owned(buf))
        }
    }

    /// Retrieves a line of text from the buffer based on the given line number.
    ///
    /// # Panics
    ///
    /// This function will panic if the `line_number` is greater than the total number
    /// of lines in the buffer's index.
    ///
    /// # Returns
    ///
    /// The line of text as a [SegStr] object.
    pub fn get_line(&self, line_number: usize) -> Option<SegStr> {
        Some(SegStr::from_bytes(self.get_bytes(line_number)?))
    }

    pub fn segment_iter(&self) -> Result<ContiguousSegmentIterator> {
        match &self.map.repr {
            BufferRepr::File { file, len, .. } => Ok(ContiguousSegmentIterator::new(
                self.index.clone(),
                0..usize::MAX, // ..self.index.line_count() if nondynamic
                BufferMap {
                    repr: BufferRepr::File {
                        file: file.try_clone()?,
                        len: *len,
                        segments: RefCell::new(LruCache::new(NonZeroUsize::new(2).unwrap())),
                    },
                    segment_size: self.map.segment_size,
                },
            )),
            BufferRepr::Stream(inner) => Ok(ContiguousSegmentIterator::new(
                self.index.clone(),
                0..usize::MAX,
                BufferMap {
                    repr: BufferRepr::Stream(RefCell::new(StreamInner {
                        pending_segs: None,
                        segments: inner.borrow().segments.clone(),
                    })),
                    segment_size: self.map.segment_size,
                },
            )),
        }
    }

    pub fn all_line_matches(&self) -> LineSet {
        LineSet::all(self.index.clone())
    }

    pub fn write_bytes<W>(&mut self, output: &mut W, lines: &LineSet) -> Result<()>
    where
        W: Write,
    {
        if !lines.is_complete() {
            return Err(crate::err::Error::InProgress);
        }

        match lines.snapshot() {
            Some(snap) => {
                let mut writer = BufWriter::new(output);
                for &ln in snap.iter() {
                    let line = self.get_bytes(ln).unwrap();
                    writer.write_all(line.as_bytes())?;
                }
            }
            None => match &mut self.map.repr {
                BufferRepr::File { file, .. } => {
                    file.seek(std::io::SeekFrom::Start(0))?;
                    let mut output = output;
                    std::io::copy(file, &mut output)?;
                }
                BufferRepr::Stream(inner) => {
                    let mut writer = BufWriter::new(output);
                    let inner = inner.borrow();

                    for seg in inner.segments.iter() {
                        writer.write_all(seg)?;
                    }
                }
            },
        }

        Ok(())
    }

    pub fn write_to_string(&mut self, output: &mut String, lines: &LineSet) -> Result<()> {
        if !lines.is_complete() {
            return Err(crate::err::Error::InProgress);
        }

        match lines.snapshot() {
            Some(snap) => {
                for &ln in snap.iter() {
                    let line = self.get_line(ln).unwrap();
                    output.push_str(line.as_str());
                }
            }
            None => match &mut self.map.repr {
                BufferRepr::File { file, .. } => {
                    file.seek(std::io::SeekFrom::Start(0))?;
                    file.read_to_string(output)?;
                }
                BufferRepr::Stream(inner) => {
                    let inner = inner.borrow();

                    for seg in inner.segments.iter() {
                        let mut reader = Cursor::new(&seg[..]);
                        reader.read_to_string(output)?;
                    }
                }
            },
        }
        output.truncate(output.trim_end().len());

        Ok(())
    }
}

pub struct ContiguousSegmentIterator {
    index: LineIndex,
    map: BufferMap,
    line_range: Range<usize>,
    // Intermediate buffer for the iterator to borrow from
    // for the case where the line crosses multiple segments
    imm_buf: Vec<u8>,
    // Intermediate segment storage for the buffer to borrow from
    // for when the buffer lies within a single segment
    imm_seg: Option<Arc<Segment>>,
}

pub struct ContiguousSegment<'a> {
    pub index: &'a LineIndex,
    pub range: Range<u64>,
    pub data: &'a [u8],
}

impl ContiguousSegmentIterator {
    fn new(index: LineIndex, line_range: Range<usize>, map: BufferMap) -> Self {
        Self {
            line_range,
            index,
            map,
            imm_buf: Vec::new(),
            imm_seg: None,
        }
    }

    #[inline]
    pub fn remaining_range(&self) -> Range<usize> {
        self.line_range.clone()
    }

    pub fn index(&self) -> &LineIndex {
        &self.index
    }

    /// Get the next buffer from the [ContiguousSegmentIterator].
    ///
    /// This function retrieves the next buffer from the `ContiguousSegmentIterator` and returns it as an `Option`.
    /// If there are no more buffers available, it returns `None`.
    ///
    /// # Returns
    ///
    /// - `Some((&Idx, u64, &[u8]))`: A tuple containing the index, starting data
    ///                               position, and a slice of the buffer data.
    /// - `None`: If there are no more buffers available.
    pub fn next(&mut self) -> Option<ContiguousSegment<'_>> {
        if self.line_range.is_empty() {
            return None;
        }

        let curr_line = self.line_range.start;
        let curr_line_data_start = self.index.data_of_line(curr_line)?;
        let curr_line_data_end = self.index.data_of_line(curr_line + 1)?;

        let curr_line_seg_start = self.map.id_of_data(curr_line_data_start);
        let curr_line_seg_end = self.map.id_of_data(curr_line_data_end);

        if curr_line_seg_end != curr_line_seg_start {
            self.imm_buf.clear();
            self.imm_buf
                .reserve((curr_line_data_end - curr_line_data_start) as usize);

            let seg_first = self.map.fetch(curr_line_seg_start)?;
            let seg_last = self.map.fetch(curr_line_seg_end)?;
            let (start, end) = (
                seg_first.translate_inner_data_index(curr_line_data_start),
                seg_last.translate_inner_data_index(curr_line_data_end),
            );

            self.imm_buf.extend_from_slice(&seg_first[start as usize..]);
            for seg_id in curr_line_seg_start + 1..curr_line_seg_end {
                self.imm_buf.extend_from_slice(&self.map.fetch(seg_id)?);
            }
            self.imm_buf.extend_from_slice(&seg_last[..end as usize]);

            self.line_range.start += 1;
            Some(ContiguousSegment {
                index: &self.index,
                range: curr_line_data_start..curr_line_data_end,
                data: &self.imm_buf,
            })
        } else {
            let curr_seg_data_start = curr_line_seg_start as u64 * self.map.segment_size;
            let curr_seg_data_end = curr_seg_data_start + self.map.segment_size;

            let line_end = match self.index.line_of_data(curr_seg_data_end) {
                Some(value) => value,
                None if self.index.report().is_complete() => self.index.line_count(),
                None => return None,
            }
            .min(self.line_range.end);
            let line_end_data_start = self.index.data_of_line(line_end)?;

            // this line should not cross multiple segments, else we would have caught in the first case
            let segment = self.map.fetch(curr_line_seg_start)?;
            let range =
                segment.translate_inner_data_range(curr_line_data_start, line_end_data_start);
            assert!(line_end_data_start - curr_seg_data_start <= self.map.segment_size);
            assert!(range.end <= self.map.segment_size);

            self.line_range.start = line_end;
            let segment = self.imm_seg.insert(segment);

            // line must end at the boundary
            Some(ContiguousSegment {
                index: &self.index,
                range: curr_line_data_start..line_end_data_start,
                data: &segment[range.start as usize..range.end as usize],
            })
        }
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use std::{
        fs::File,
        io::{BufReader, Read},
        num::NonZeroUsize,
    };

    use crate::buf::SegBuffer;

    #[test]
    fn file_stream_consistency_1() -> Result<()> {
        file_stream_consistency_base(File::open("../../tests/test_10.log")?, 10)
    }

    #[test]
    fn file_stream_consistency_2() -> Result<()> {
        file_stream_consistency_base(File::open("../../tests/test_50_long.log")?, 50)
    }

    #[test]
    fn file_stream_consistency_3() -> Result<()> {
        file_stream_consistency_base(File::open("../../tests/test_5000000.log")?, 5_000_000)
    }

    fn file_stream_consistency_base(file: File, line_count: usize) -> Result<()> {
        let stream = BufReader::new(file.try_clone()?);

        let file_index = SegBuffer::read_file(file, NonZeroUsize::new(25).unwrap(), true)?;
        let stream_index = SegBuffer::read_stream(Box::new(stream), true)?;

        assert_eq!(file_index.line_count(), stream_index.line_count());
        assert_eq!(file_index.line_count(), line_count);
        for i in 0..file_index.line_count() {
            assert_eq!(
                file_index.get_line(i).unwrap().as_str(),
                stream_index.get_line(i).unwrap().as_str()
            );
        }

        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn multi_buffer_consistency_1() -> Result<()> {
        multi_buffer_consistency_base(File::open("../../tests/test_10.log")?)
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn multi_buffer_consistency_2() -> Result<()> {
        multi_buffer_consistency_base(File::open("../../tests/test_50_long.log")?)
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn multi_buffer_consistency_3() -> Result<()> {
        multi_buffer_consistency_base(File::open("../../tests/test_5000000.log")?)
    }

    fn multi_buffer_consistency_base(file: File) -> Result<()> {
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file.try_clone()?);

        let file_buffer = SegBuffer::read_file(file, NonZeroUsize::new(25).unwrap(), true)?;
        let mut buffers = file_buffer.segment_iter()?;

        let mut total_bytes = 0;
        let mut validate_buf = Vec::new();
        while let Some(segment) = buffers.next() {
            // Validate that the specialized slice reader and normal sequential reads are consistent
            assert_eq!(segment.range.start, total_bytes);
            total_bytes += segment.data.len() as u64;
            validate_buf.resize(segment.data.len(), 0);
            reader.read_exact(&mut validate_buf)?;
            assert_eq!(segment.data, validate_buf);
        }
        assert_eq!(total_bytes, file_len);

        Ok(())
    }
}
