use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};

use crate::*;

#[derive(Serialize, Deserialize)]
struct KvRecord {
    key: String,
    value: String,
}

impl<const LEAF_FANOUT: usize> Tree<LEAF_FANOUT> {
    pub fn export_jsonl<P: AsRef<Path>>(&self, path: P) -> std::io::Result<usize> {
        self.check_error()?;

        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        let mut count: usize = 0;

        for kv_res in self.iter() {
            let (key, value) = kv_res?;
            let record = KvRecord {
                key: BASE64.encode(key.as_ref()),
                value: BASE64.encode(value.as_ref()),
            };
            serde_json::to_writer(&mut writer, &record).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;
            writer.write_all(b"\n")?;
            count += 1;
        }

        writer.flush()?;
        Ok(count)
    }

    pub fn import_jsonl<P: AsRef<Path>>(&self, path: P) -> std::io::Result<usize> {
        self.check_error()?;

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut batch = Batch::default();
        let mut count: usize = 0;
        const BATCH_SIZE: usize = 1000;

        for line_res in reader.lines() {
            let line = line_res?;
            if line.trim().is_empty() {
                continue;
            }
            let record: KvRecord =
                serde_json::from_str(&line).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
                })?;
            let key_bytes = BASE64.decode(&record.key).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;
            let value_bytes = BASE64.decode(&record.value).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;
            batch.insert(key_bytes, value_bytes);
            count += 1;

            if count % BATCH_SIZE == 0 {
                let batch_to_apply = std::mem::take(&mut batch);
                self.apply_batch(batch_to_apply)?;
            }
        }

        if !batch.writes.is_empty() {
            self.apply_batch(batch)?;
        }

        Ok(count)
    }
}
