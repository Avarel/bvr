use bvr::file::index::{sync::AsyncIndex, FileIndex};

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let file = rt.block_on(tokio::fs::File::open("./log_generated.log")).unwrap();

    let start = std::time::Instant::now();
    let index = rt.block_on(AsyncIndex::new_complete(&file)).unwrap();
    dbg!(index.line_count());

    // let file = rt.block_on(file.into_std());
    // let start = std::time::Instant::now();
    // let index = IncompleteIndex::new().index(&file).unwrap();
    // dbg!(index.line_count());

    let elapsed = start.elapsed();
    println!("{}s", elapsed.as_secs_f64());
}