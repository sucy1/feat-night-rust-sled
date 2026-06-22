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
            let line = serde_json::to_string(&record).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;
            writeln!(writer, "{}", line)?;
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

#[cfg(test)]
mod tests {
    use crate::Config;

    type Db = crate::Db<1024>;

    fn tmp_config() -> Config {
        Config::tmp().unwrap().flush_every_ms(None)
    }

    #[test]
    fn export_import_roundtrip() {
        let _ = env_logger::try_init();

        let export_path =
            std::env::temp_dir().join(format!("sled_export_test_{}", std::process::id()));

        let original_data: Vec<(Vec<u8>, Vec<u8>)> = (0..100u32)
            .map(|i| {
                (
                    format!("key_{:04}", i).into_bytes(),
                    format!("value_{:04}_data", i).into_bytes(),
                )
            })
            .collect();

        let db1: Db = tmp_config().open().unwrap();
        for (k, v) in &original_data {
            db1.insert(k.as_slice(), v.as_slice()).unwrap();
        }
        assert_eq!(db1.iter().count(), 100);

        let exported = db1.export_jsonl(&export_path).unwrap();
        assert_eq!(exported, 100);

        let db2: Db = tmp_config().open().unwrap();
        assert_eq!(db2.iter().count(), 0);

        let imported = db2.import_jsonl(&export_path).unwrap();
        assert_eq!(imported, 100);

        assert_eq!(db2.iter().count(), 100);
        for (k, v) in &original_data {
            let got = db2.get(k).unwrap().unwrap();
            assert_eq!(got.as_ref(), &v[..]);
        }

        let mut iter1 = db1.iter();
        let mut iter2 = db2.iter();
        loop {
            match (iter1.next(), iter2.next()) {
                (Some(a), Some(b)) => {
                    let (ak, av) = a.unwrap();
                    let (bk, bv) = b.unwrap();
                    assert_eq!(ak, bk);
                    assert_eq!(av, bv);
                }
                (None, None) => break,
                _ => panic!("iterator length mismatch"),
            }
        }

        let _ = std::fs::remove_file(&export_path);
    }

    #[test]
    fn export_import_empty() {
        let _ = env_logger::try_init();

        let export_path = std::env::temp_dir().join(format!(
            "sled_export_empty_test_{}",
            std::process::id()
        ));

        let db1: Db = tmp_config().open().unwrap();
        let exported = db1.export_jsonl(&export_path).unwrap();
        assert_eq!(exported, 0);

        let db2: Db = tmp_config().open().unwrap();
        let imported = db2.import_jsonl(&export_path).unwrap();
        assert_eq!(imported, 0);
        assert_eq!(db2.iter().count(), 0);

        let _ = std::fs::remove_file(&export_path);
    }

    #[test]
    fn export_import_binary_data() {
        let _ = env_logger::try_init();

        let export_path = std::env::temp_dir().join(format!(
            "sled_export_binary_test_{}",
            std::process::id()
        ));

        let db1: Db = tmp_config().open().unwrap();

        let binary_key: Vec<u8> = (0u8..=255).collect();
        let binary_value: Vec<u8> = (255u8..=0).rev().collect();
        db1.insert(binary_key.as_slice(), binary_value.as_slice()).unwrap();

        db1.insert(&[0x00, 0x01, 0xFF, 0xFE], &[0xDE, 0xAD, 0xBE, 0xEF][..])
            .unwrap();

        let exported = db1.export_jsonl(&export_path).unwrap();
        assert_eq!(exported, 2);

        let db2: Db = tmp_config().open().unwrap();
        let imported = db2.import_jsonl(&export_path).unwrap();
        assert_eq!(imported, 2);

        let v1 = db2.get(&binary_key).unwrap().unwrap();
        assert_eq!(v1.as_ref(), &binary_value[..]);

        let v2 = db2.get(&[0x00, 0x01, 0xFF, 0xFE]).unwrap().unwrap();
        assert_eq!(v2.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF][..]);

        let _ = std::fs::remove_file(&export_path);
    }

    #[test]
    fn export_import_large_batch() {
        let _ = env_logger::try_init();

        let export_path = std::env::temp_dir().join(format!(
            "sled_export_large_test_{}",
            std::process::id()
        ));

        let db1: Db = tmp_config().open().unwrap();

        let count = 2500usize;
        for i in 0..count {
            let k = (i as u64).to_be_bytes().to_vec();
            let v = vec![(i % 256) as u8; 64];
            db1.insert(k.as_slice(), v.as_slice()).unwrap();
        }
        assert_eq!(db1.iter().count(), count);

        let exported = db1.export_jsonl(&export_path).unwrap();
        assert_eq!(exported, count);

        let db2: Db = tmp_config().open().unwrap();
        let imported = db2.import_jsonl(&export_path).unwrap();
        assert_eq!(imported, count);
        assert_eq!(db2.iter().count(), count);

        for i in 0..count {
            let k = (i as u64).to_be_bytes().to_vec();
            let expected = vec![(i % 256) as u8; 64];
            let got = db2.get(&k).unwrap().unwrap();
            assert_eq!(got.as_ref(), &expected[..]);
        }

        let _ = std::fs::remove_file(&export_path);
    }

    #[test]
    fn export_import_to_same_tree() {
        let _ = env_logger::try_init();

        let export_path = std::env::temp_dir().join(format!(
            "sled_export_same_tree_test_{}",
            std::process::id()
        ));

        let db: Db = tmp_config().open().unwrap();
        db.insert(b"key1", b"value1").unwrap();
        db.insert(b"key2", b"value2").unwrap();

        let exported = db.export_jsonl(&export_path).unwrap();
        assert_eq!(exported, 2);

        db.insert(b"key2", b"value2_modified").unwrap();
        db.insert(b"key3", b"value3").unwrap();

        let imported = db.import_jsonl(&export_path).unwrap();
        assert_eq!(imported, 2);

        let v1 = db.get(b"key1").unwrap().unwrap();
        assert_eq!(v1.as_ref(), b"value1");

        let v2 = db.get(b"key2").unwrap().unwrap();
        assert_eq!(v2.as_ref(), b"value2");

        let v3 = db.get(b"key3").unwrap().unwrap();
        assert_eq!(v3.as_ref(), b"value3");

        let _ = std::fs::remove_file(&export_path);
    }
}
