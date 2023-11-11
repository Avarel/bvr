# BVR Core

This is the core library powering the `bvr` pager. It contains `ShardedFile` and `AsyncIndex`,
which are the fundamental building blocks behind how `bvr` works.

`ShardedFile` breaks down the file into shards, which can be loaded into memory on demand.
* For streamed input, they are funneled into anonymous mmap pages.

`InflightIndex` contains the logic for indexing files for fast random access of lines.
* The index can be accessed from non-async context for cheap costs, and is the primary
  way of interacting with it.
* An async runtime is required to drive the indexing process and a reader can cheaply read it
  at the same time, as the index's internal vector is append only, atomically refcounted,
  and copy-on-write.