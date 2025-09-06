use anyhow::Result;
use rand::{distr::Distribution, rngs::SmallRng, Rng, SeedableRng};
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
    generate_log_file("tests/test_5000000_long.log", 5_000_000, 50..1500, 4)?;
    Ok(())
}

struct Charset;
impl Distribution<u8> for Charset {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u8 {
        const GEN_ASCII_STR_CHARSET: &[u8] = b"         \
                \t\t\t\t\t\
                ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                abcdefghijklmnopqrstuvwxyz\
                0123456789\
                !@#$%^&*()_+-=[]{}|;:'\",./<>?";
        const RANGE: u32 = GEN_ASCII_STR_CHARSET.len() as u32;
        loop {
            let var = rng.next_u32() >> (32 - 7);
            if var < RANGE {
                return GEN_ASCII_STR_CHARSET[var as usize];
            }
        }
    }
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
            .sample_iter(Charset)
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
