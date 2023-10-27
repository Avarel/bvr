use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Seek, Write},
};

use anyhow::{anyhow, Result};

use crate::rg::de::RgMessage;

pub struct RgIndex {
    start: u64,
    end: u64,
}

pub struct RgFile {
    name: String,
    file: File,
    index: Vec<RgIndex>,
}

impl RgFile {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            file: tempfile::tempfile().unwrap(),
            index: Vec::new(),
        }
    }
}

pub fn read_messages<R: Read>(rdr: R) -> Result<Vec<RgFile>> {
    let reader = BufReader::new(rdr);

    let mut files = Vec::new();

    let mut file = RgFile::new();
    let mut writer = BufWriter::new(&mut file.file);
    for (_, line) in reader.lines().enumerate() {
        let line = line?;
        let rg_msg: RgMessage =
            serde_json::from_str(&line).map_err(|e| anyhow!("Failed to parse JSON: {}", e))?;

        let start = writer.stream_position()?;
        writer.write(line.as_bytes())?;
        let end = writer.stream_position()?;

        file.index.push(RgIndex { start, end });

        match rg_msg {
            RgMessage::Begin { path } => {
                file.name = path.to_string();
            },
            RgMessage::End { path, .. } => {
                drop(writer);
                files.push(file);
                file = RgFile::new();
                writer = BufWriter::new(&mut file.file);
            },
            _ => ()
        }
    }

    Ok(files)
}
