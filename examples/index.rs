use dltwf::file::ShardedFile;

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let file = rt.block_on(tokio::fs::File::open("./log_generated.log")).unwrap();

    let start = std::time::Instant::now();

    let file = rt.block_on(ShardedFile::new(file, 25)).unwrap();
    dbg!(file.line_count());

    let elapsed = start.elapsed();
    println!("{}s", elapsed.as_secs_f64());
}