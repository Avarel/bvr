use std::{fs::OpenOptions, io::BufWriter, io::{Write, stdout}, ops::Range};

use rand::{distributions::Alphanumeric, Rng};

fn main() {
    const LINES: usize = 5_000_000;
    const CHARS_PER_LINE: Range<usize> = 50..150;

    let mut rng = rand::thread_rng();

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open("./log_generated.log")
        .unwrap();

    let mut stdout = stdout().lock();
    let mut writer = BufWriter::new(file);
    
    for i in 0..LINES {
        let len = rng.gen_range(CHARS_PER_LINE);

        let s = (&mut rng)
            .sample_iter(Alphanumeric)
            .map(char::from)
            .take(len)
            .collect::<String>();

        let timestamp = time::OffsetDateTime::now_utc().to_string();
        writeln!(writer, "{} {}", timestamp, s).unwrap();

        if i % 100_000 == 0 {
            write!(stdout, "\rWrote {i} lines").unwrap();
            stdout.flush().unwrap();
        }
    }
    stdout.flush().unwrap();
    writeln!(stdout).unwrap();
}
