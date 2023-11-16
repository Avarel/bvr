use std::{rc::Rc, borrow::Cow, ops::Range};
use crate::{buf::shard::Shard, index::BufferIndex};

pub struct ShardMultiLineBuffer<Idx> {
    index: Idx,
    shards: Vec<Rc<Shard>>,
    curr_line: usize
}

impl<Idx> ShardMultiLineBuffer<Idx> where Idx: BufferIndex {
    fn next(&mut self) -> Option<(Range<usize>, Cow<[u8]>)> {
        let curr_line = self.curr_line;
        if curr_line < self.index.line_count() {
            let curr_line_data_start = self.index.data_of_line(curr_line).unwrap();
            let curr_line_data_end = self.index.data_of_line(curr_line + 1).unwrap();

            let curr_line_shard_start = (curr_line_data_start / crate::SHARD_SIZE) as usize;
            let curr_line_shard_end = (curr_line_data_end / crate::SHARD_SIZE) as usize;

            if curr_line_shard_end != curr_line_shard_start {
                let mut buf = Vec::with_capacity((curr_line_data_end - curr_line_data_start) as usize);

                let shard_first = &self.shards[curr_line_shard_start];
                let shard_last = &self.shards[curr_line_shard_end];
                let (start, end) = (
                    shard_first.translate_inner_data_index(curr_line_data_start),
                    shard_last.translate_inner_data_index(curr_line_data_end),
                );

                buf.extend_from_slice(&shard_first[start as usize..]);
                for shard_id in curr_line_shard_start + 1..curr_line_shard_end {
                    buf.extend_from_slice(&self.shards[shard_id]);
                }
                buf.extend_from_slice(&shard_last[..end as usize]);

                self.curr_line += 1;
                return Some((curr_line..curr_line + 1, Cow::Owned(buf)));
            } else {
                let curr_shard_data_start = curr_line_shard_start as u64 * crate::SHARD_SIZE;
                let curr_shard_data_end = curr_shard_data_start + crate::SHARD_SIZE;
                
                let line_end = self.index.line_of_data(curr_shard_data_end).unwrap_or_else(|| {
                    self.index.line_count()
                });
                let line_end_data_start = self.index.data_of_line(line_end).unwrap();

                // this line should not cross multiple shards, else we would have caught in the first case
                let (start, end) = self.shards[curr_line_shard_start].translate_inner_data_range(curr_line_data_start, line_end_data_start);
                assert!(line_end_data_start - curr_shard_data_start <= crate::SHARD_SIZE);
                assert!(end <= crate::SHARD_SIZE);

                self.curr_line = line_end;
                // line must end at the boundary
                return Some((curr_line..line_end, Cow::Borrowed(&self.shards[curr_line_shard_start][start as usize..end as usize])))
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::{BufReader, Read}};
    use anyhow::Result;
    use crate::{ShardedBuffer, index::CompleteIndex};
    use super::ShardMultiLineBuffer;

    #[test]
    fn search_buffer_consistency_1() -> Result<()> {
        search_buffer_consistency_base(File::open("../../tests/test_10.log")?, 10)
    }

    #[test]
    fn search_buffer_consistency_2() -> Result<()> {
        search_buffer_consistency_base(File::open("../../tests/test_50_long.log")?, 50)
    }

    #[test]
    fn search_buffer_consistency_3() -> Result<()> {
        search_buffer_consistency_base(File::open("../../tests/test_5000000.log")?, 5_000_000)
    }

    fn search_buffer_consistency_base(file: File, line_count: usize) -> Result<()> {
        let mut reader = BufReader::new(file.try_clone()?);

        let mut file_index = ShardedBuffer::<CompleteIndex>::read_file(file, 25)?;
        let shards = file_index.render_shards()?;
        let index = file_index.index().clone();

        let mut searcher = ShardMultiLineBuffer {
            index,
            shards,
            curr_line: 0
        };

        let mut total_lines = 0;
        let mut validate_buf = Vec::new();
        while let Some((lines, buf)) = searcher.next() {
            // Validate that the specialized slice reader and normal sequential reads are consistent
            total_lines += lines.end - lines.start;
            validate_buf.resize(buf.len(), 0);
            reader.read_exact(&mut validate_buf)?;
            assert_eq!(buf, validate_buf);
        }
        assert_eq!(total_lines, line_count);

        Ok(())
    }
}