# BVR Core

This crate contains the core functionality of the BVR pager.

## Components
### Segment Buffers
These buffers are used to store and interact with line-indexed data. The buffers
can be created from files or streams.

#### Segment
Segment buffers are divided into segments. Segments are the smallest unit of
data loaded into memory. They are currently configured to be 1MB in size.
* For files, segments are loaded into memory on demand, are unloaded based on an LRU cache.
* For streams, all segments are loaded into memory.

#### SegBytes, SegStr
Data from the segment buffers is accessed through the `SegBytes` and `SegStr`.
They borrow and pin the segment, preventing it from being unloaded from memory.

### Index
The `LineIndex` is used to map between line numbers and byte offsets. It is primarily
used to answer questions like "what line is at this byte offset?" and "what byte
offset is at this line number?".

### Matches
The `LineMatches` is used to store matches in iteration order for a particular
regex upon a buffer. They can be composed into a single `LineMatches`.
