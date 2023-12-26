use crate::{
    cowvec::{CowVec, CowVecWriter},
    err::Result,
    LineMatches,
};

struct QueueMatch {
    matches: LineMatches,
    index: usize,
}

struct Queues {
    queues: Vec<QueueMatch>,
}

impl Queues {
    fn new(queues: Vec<LineMatches>) -> Self {
        Self {
            queues: queues
                .into_iter()
                .map(|queue| QueueMatch {
                    matches: queue,
                    index: 0,
                })
                .collect(),
        }
    }

    fn take_lowest(&mut self) -> Option<usize> {
        // Take the lowest line number from all the queues
        // and progress the queue that yielded the lowest line number
        'outer: loop {
            let mut min = None;
            for queue in &mut self.queues {
                // We reached the end of this queue but its not complete
                // It could yield a lower line number than all of the other queues
                if queue.index >= queue.matches.len() && !queue.matches.is_complete() {
                    continue 'outer;
                }
                if let Some(ln) = queue.matches.get(queue.index) {
                    if let Some((_, min_ln)) = min {
                        if ln < min_ln {
                            min = Some((&mut queue.index, ln));
                        }
                    } else {
                        min = Some((&mut queue.index, ln));
                    }
                }
            }

            return if let Some((offset, line_number)) = min {
                *offset += 1;
                Some(line_number)
            } else {
                assert!(self.queues.iter().all(|queue| queue.matches.is_complete()));
                None
            };
        }
    }
}

struct LineCompositeRemote {
    buf: CowVecWriter<usize>,
}

impl LineCompositeRemote {
    pub fn compute(mut self, filters: Vec<LineMatches>) -> Result<()> {
        let len = filters.iter().map(|filter| filter.len()).sum::<usize>();
        // Divide by 2 because there may be a lot of overlap, but hopefully this
        // nonscientific guess is good enough
        // In the common case, we only have 1 reallocation so its not too bad
        self.buf.reserve(len / 2);

        let mut queues = Queues::new(filters);

        while let Some(line_number) = queues.take_lowest() {
            if !self.buf.has_readers() {
                break;
            }

            if let Some(&last) = self.buf.last() {
                if last == line_number {
                    continue;
                }
                debug_assert!(line_number > last);
            }
            self.buf.push(line_number);
        }
        Ok(())
    }
}

impl LineComposite {
    #[inline]
    pub fn new(filters: Vec<LineMatches>) -> Self {
        let (buf, writer) = CowVec::new();
        std::thread::spawn(move || LineCompositeRemote { buf: writer }.compute(filters));
        Self { buf }
    }

    #[inline]
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        self.buf.get(index)
    }
}

pub struct LineComposite {
    buf: CowVec<usize>,
}
