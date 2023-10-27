use anyhow::Result;
use tokio::fs::File;
use tokio::sync::mpsc::Receiver;

pub(crate) type FileIndex = Vec<u64>;

pub(crate) struct IndexingTask {
    pub(crate) sx: tokio::sync::mpsc::Sender<u64>,
    pub(crate) data: memmap2::Mmap,
    pub(crate) start: u64,
}

impl IndexingTask {
    pub(crate) async fn new(
        file: &File,
        start: u64,
        end: u64,
    ) -> Result<(Self, Receiver<u64>)> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        let (sx, rx) = tokio::sync::mpsc::channel(1 << 10);
        Ok((Self { sx, data, start }, rx))
    }

    pub(crate) async fn worker(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.data) {
            self.sx.send(self.start + i as u64).await?;
        }

        Ok(())
    }
}

pub(crate) async fn index_file(file: &File, len: u64) -> Result<FileIndex> {
    let (sx, mut rx) = tokio::sync::mpsc::channel(10);

    let file = file.try_clone().await?;

    let spawner = tokio::task::spawn(async move {
        const SIZE: u64 = 1 << 20;
        let mut curr = 0;

        while curr < len {
            let end = (curr + SIZE).min(len);
            let (task, task_rx) = IndexingTask::new(&file, curr, end).await?;
            sx.send(task_rx).await?;
            tokio::task::spawn(task.worker());

            curr = end;
        }

        Ok::<(), anyhow::Error>(())
    });

    let mut result = FileIndex::new();
    result.push(0);

    while let Some(mut task_rx) = rx.recv().await {
        while let Some(v) = task_rx.recv().await {
            result.push(v);
        }
    }

    result.push(len);

    spawner.await??;

    Ok(result)
}
