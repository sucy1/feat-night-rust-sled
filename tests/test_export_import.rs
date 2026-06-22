mod common;

use std::fs;
use std::io::Write;

use sled::{Config, Db as SledDb};

type Db = SledDb<3>;

fn tmp_config() -> Config {
    Config::tmp().unwrap().flush_every_ms(None)
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "sled_{}_{}",
        name,
        std::process::id()
    ));
    p
}

#[test]
fn roundtrip_100_kv() {
    common::setup_logger();

    let path = tmp_path("roundtrip_100");
    let export_path = tmp_path("roundtrip_100_jsonl");

    let db: Db = tmp_config().open().unwrap();
    for i in 0_u32..100 {
        let k = (i as u64).to_be_bytes();
        let v = (i as u64).to_le_bytes();
        db.insert(&k, &v).unwrap();
    }
    assert_eq!(db.iter().count(), 100);

    let exported = db.export_jsonl(&export_path).unwrap();
    assert_eq!(exported, 100);

    let db2: Db = tmp_config().open().unwrap();
    let imported = db2.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, 100);

    for i in 0_u32..100 {
        let k = (i as u64).to_be_bytes();
        let expected = (i as u64).to_le_bytes();
        let got = db2.get(&k).unwrap().unwrap();
        assert_eq!(got.as_ref(), &expected[..]);
    }

    let _ = fs::remove_file(&export_path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn roundtrip_empty_db() {
    common::setup_logger();

    let export_path = tmp_path("empty_jsonl");

    let db: Db = tmp_config().open().unwrap();
    assert_eq!(db.iter().count(), 0);

    let exported = db.export_jsonl(&export_path).unwrap();
    assert_eq!(exported, 0);
    assert!(export_path.exists());

    let content = fs::read_to_string(&export_path).unwrap();
    assert!(content.is_empty());

    let db2: Db = tmp_config().open().unwrap();
    let imported = db2.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, 0);
    assert_eq!(db2.iter().count(), 0);

    let _ = fs::remove_file(&export_path);
}

#[test]
fn roundtrip_large_values() {
    common::setup_logger();

    let export_path = tmp_path("large_values_jsonl");

    let db: Db = tmp_config().open().unwrap();

    let big_key = vec![0xABu8; 1024];
    let big_value = vec![0xCDu8; 8192];
    db.insert(big_key.as_slice(), big_value.as_slice()).unwrap();

    let exported = db.export_jsonl(&export_path).unwrap();
    assert_eq!(exported, 1);

    let db2: Db = tmp_config().open().unwrap();
    let imported = db2.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, 1);

    let got = db2.get(&big_key).unwrap().unwrap();
    assert_eq!(got.as_ref(), &big_value[..]);
    assert_eq!(got.len(), 8192);

    let _ = fs::remove_file(&export_path);
}

#[test]
fn roundtrip_binary_special_bytes() {
    common::setup_logger();

    let export_path = tmp_path("special_bytes_jsonl");

    let db: Db = tmp_config().open().unwrap();

    for i in 0..=255u8 {
        let k = vec![i, i.wrapping_add(1), i.wrapping_add(2)];
        let v = vec![i; 1];
        db.insert(k.as_slice(), v.as_slice()).unwrap();
    }
    assert_eq!(db.iter().count(), 256);

    let exported = db.export_jsonl(&export_path).unwrap();
    assert_eq!(exported, 256);

    let db2: Db = tmp_config().open().unwrap();
    let imported = db2.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, 256);

    for i in 0..=255u8 {
        let k = vec![i, i.wrapping_add(1), i.wrapping_add(2)];
        let expected = vec![i; 1];
        let got = db2.get(&k).unwrap().unwrap();
        assert_eq!(got.as_ref(), &expected[..]);
    }

    let _ = fs::remove_file(&export_path);
}

#[test]
fn export_file_format_jsonl() {
    common::setup_logger();

    let export_path = tmp_path("file_format_jsonl");

    let db: Db = tmp_config().open().unwrap();
    db.insert(b"hello", b"world").unwrap();
    db.insert(b"foo", b"bar").unwrap();

    db.export_jsonl(&export_path).unwrap();

    let content = fs::read_to_string(&export_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    for line in lines {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(parsed.is_object());
        let obj = parsed.as_object().unwrap();
        assert!(obj.contains_key("key"));
        assert!(obj.contains_key("value"));
        assert!(obj["key"].is_string());
        assert!(obj["value"].is_string());
    }

    let _ = fs::remove_file(&export_path);
}

#[test]
fn import_malformed_line_returns_error() {
    common::setup_logger();

    let export_path = tmp_path("malformed_jsonl");

    {
        let mut f = fs::File::create(&export_path).unwrap();
        writeln!(f, r#"{{"key": "aGVsbG8=", "value": "d29ybGQ="}}"#).unwrap();
        writeln!(f, "not valid json at all!!!").unwrap();
    }

    let db: Db = tmp_config().open().unwrap();
    let result = db.import_jsonl(&export_path);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);

    let _ = fs::remove_file(&export_path);
}

#[test]
fn import_invalid_base64_returns_error() {
    common::setup_logger();

    let export_path = tmp_path("invalid_b64_jsonl");

    {
        let mut f = fs::File::create(&export_path).unwrap();
        writeln!(f, r#"{{"key": "!!!not-base64!!!", "value": "d29ybGQ="}}"#).unwrap();
    }

    let db: Db = tmp_config().open().unwrap();
    let result = db.import_jsonl(&export_path);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);

    let _ = fs::remove_file(&export_path);
}

#[test]
fn import_missing_value_field_returns_error() {
    common::setup_logger();

    let export_path = tmp_path("missing_field_jsonl");

    {
        let mut f = fs::File::create(&export_path).unwrap();
        writeln!(f, r#"{{"key": "aGVsbG8="}}"#).unwrap();
    }

    let db: Db = tmp_config().open().unwrap();
    let result = db.import_jsonl(&export_path);
    assert!(result.is_err());

    let _ = fs::remove_file(&export_path);
}

#[test]
fn import_nonexistent_file_returns_error() {
    common::setup_logger();

    let nonexistent = tmp_path("nonexistent_jsonl");
    let _ = fs::remove_file(&nonexistent);

    let db: Db = tmp_config().open().unwrap();
    let result = db.import_jsonl(&nonexistent);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn import_overwrites_existing_keys() {
    common::setup_logger();

    let export_path = tmp_path("overwrite_jsonl");

    let db: Db = tmp_config().open().unwrap();
    db.insert(b"a", b"original").unwrap();
    db.insert(b"b", b"keep_this").unwrap();

    let db_export: Db = tmp_config().open().unwrap();
    db_export.insert(b"a", b"overwritten").unwrap();
    db_export.export_jsonl(&export_path).unwrap();

    let imported = db.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, 1);

    let got_a = db.get(b"a").unwrap().unwrap();
    assert_eq!(got_a.as_ref(), b"overwritten");

    let got_b = db.get(b"b").unwrap().unwrap();
    assert_eq!(got_b.as_ref(), b"keep_this");

    let _ = fs::remove_file(&export_path);
}

#[test]
fn many_records_batch_boundary() {
    common::setup_logger();

    let export_path = tmp_path("batch_boundary_jsonl");

    let count = 2500usize;

    let db: Db = tmp_config().open().unwrap();
    for i in 0..count {
        let k = (i as u32).to_be_bytes();
        let v = (i as u32).to_le_bytes();
        db.insert(&k, &v).unwrap();
    }

    let exported = db.export_jsonl(&export_path).unwrap();
    assert_eq!(exported, count);

    let db2: Db = tmp_config().open().unwrap();
    let imported = db2.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, count);
    assert_eq!(db2.iter().count(), count);

    for i in 0..count {
        let k = (i as u32).to_be_bytes();
        let expected = (i as u32).to_le_bytes();
        let got = db2.get(&k).unwrap().unwrap();
        assert_eq!(got.as_ref(), &expected[..]);
    }

    let _ = fs::remove_file(&export_path);
}

#[test]
fn separate_tree_export_import() {
    common::setup_logger();

    let export_path = tmp_path("separate_tree_jsonl");

    let db: Db = tmp_config().open().unwrap();

    let tree = db.open_tree(b"my_tree").unwrap();
    tree.insert(b"tk1", b"tv1").unwrap();
    tree.insert(b"tk2", b"tv2").unwrap();
    assert_eq!(tree.iter().count(), 2);

    let exported = tree.export_jsonl(&export_path).unwrap();
    assert_eq!(exported, 2);

    let db2: Db = tmp_config().open().unwrap();
    let tree2 = db2.open_tree(b"imported_tree").unwrap();
    let imported = tree2.import_jsonl(&export_path).unwrap();
    assert_eq!(imported, 2);

    let got1 = tree2.get(b"tk1").unwrap().unwrap();
    assert_eq!(got1.as_ref(), b"tv1");
    let got2 = tree2.get(b"tk2").unwrap().unwrap();
    assert_eq!(got2.as_ref(), b"tv2");

    assert_eq!(db2.iter().count(), 0);

    let _ = fs::remove_file(&export_path);
}
