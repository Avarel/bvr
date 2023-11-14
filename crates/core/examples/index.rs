use bvr_core::index::{inflight::InflightIndex, BufferIndex};

fn main() {
    let file = std::fs::File::open("./Cargo.toml").unwrap();

    let start = std::time::Instant::now();
    let index = InflightIndex::new_complete(&file).unwrap();
    dbg!(index.line_count());

    let elapsed = start.elapsed();
    println!("{}s", elapsed.as_secs_f64());
}