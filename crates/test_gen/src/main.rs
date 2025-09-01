use anyhow::Result;
use rand::{distr::Alphanumeric, rngs::SmallRng, Rng, SeedableRng};
use std::{
    fs::OpenOptions,
    io::BufWriter,
    io::{stdout, Write},
    ops::Range,
};

fn main() -> Result<()> {
    std::fs::create_dir("tests").ok();
    generate_log_file("tests/test_10.log", 10, 50..150, 1)?;
    generate_log_file("tests/test_50_long.log", 50, 9000..15000, 2)?;
    generate_log_file("tests/test_5000000.log", 5_000_000, 50..150, 3)?;
    Ok(())
}

fn generate_log_file(
    path: &str,
    lines: usize,
    chars_per_line: Range<usize>,
    seed: u64,
) -> Result<()> {
    let mut rng = SmallRng::seed_from_u64(seed);

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();

    let mut stdout = stdout().lock();
    let mut writer = BufWriter::new(file);

    for i in 0..lines {
        let len = rng.random_range(chars_per_line.clone());

        let s = (&mut rng)
            .sample_iter(Alphanumeric)
            .map(char::from)
            .take(len)
            .collect::<String>();

        let timestamp = time::OffsetDateTime::now_utc().to_string();
        if i != 0 {
            writer.write_all(b"\n")?;
        }
        write!(writer, "{} {}", timestamp, s)?;

        if i % 100_000 == 0 {
            write!(stdout, "\r{path}: Wrote {i} lines")?;
            stdout.flush().unwrap();
        }
    }
    write!(stdout, "\r{path}: Wrote {lines} lines")?;
    stdout.flush()?;
    writeln!(stdout)?;
    Ok(())
}
