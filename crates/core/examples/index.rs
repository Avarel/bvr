use bvr_core::index::{inflight::InflightIndex, BufferIndex};

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let file = rt.block_on(tokio::fs::File::open("./log_generated.log")).unwrap();

    let start = std::time::Instant::now();
    let index = rt.block_on(InflightIndex::new_complete(&file)).unwrap();
    dbg!(index.line_count());

    let elapsed = start.elapsed();
    println!("{}s", elapsed.as_secs_f64());
}