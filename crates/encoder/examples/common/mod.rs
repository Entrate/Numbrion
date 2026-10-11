//! Raw array directories shared with `parity/arrays.py`: `meta.json` ({name: [dtype, shape]}) plus one C-order,
//! little-endian `<name>.bin` per array. Used by the parity examples only.
#![allow(dead_code)]

use encoder::json::Json;
use std::{fs, path::Path};

pub struct Array {
    pub dtype: String,
    pub shape: Vec<usize>,
    pub bytes: Vec<u8>,
}

impl Array {
    pub fn len(&self) -> usize {
        self.shape.iter().product()
    }

    pub fn f32(&self) -> Vec<f32> {
        assert_eq!(self.dtype, "float32");
        self.bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
    }

    pub fn i64(&self) -> Vec<i64> {
        assert_eq!(self.dtype, "int64");
        self.bytes.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect()
    }

    pub fn i32(&self) -> Vec<i32> {
        assert_eq!(self.dtype, "int32");
        self.bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect()
    }

    pub fn bools(&self) -> Vec<bool> {
        assert_eq!(self.dtype, "bool");
        self.bytes.iter().map(|&b| b != 0).collect()
    }
}

pub fn load(dir: &Path) -> std::collections::HashMap<String, Array> {
    let meta = Json::parse(&fs::read_to_string(dir.join("meta.json")).expect("meta.json")).expect("meta.json");
    let mut out = std::collections::HashMap::new();
    for (name, spec) in meta.members() {
        let dtype = spec.items()[0].str_or_empty().to_string();
        let shape = spec.items()[1].items().iter().map(|x| x.as_f64().unwrap() as usize).collect();
        let bytes = fs::read(dir.join(format!("{name}.bin"))).expect("array file");
        out.insert(name.clone(), Array { dtype, shape, bytes });
    }
    out
}

/// Writes `meta.json` from scratch with every array of `arrays`.
pub struct Writer {
    dir: std::path::PathBuf,
    meta: Vec<String>,
}

impl Writer {
    pub fn new(dir: &Path) -> Self {
        fs::create_dir_all(dir).expect("output directory");
        Writer { dir: dir.to_path_buf(), meta: Vec::new() }
    }

    fn put(&mut self, name: &str, dtype: &str, shape: &[usize], bytes: Vec<u8>) {
        assert_eq!(bytes.len() % shape.iter().product::<usize>().max(1), 0);
        fs::write(self.dir.join(format!("{name}.bin")), bytes).expect("write array");
        let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
        self.meta.push(format!("\"{name}\": [\"{dtype}\", [{}]]", dims.join(", ")));
    }

    pub fn f32(&mut self, name: &str, shape: &[usize], v: &[f32]) {
        self.put(name, "float32", shape, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }

    pub fn i64(&mut self, name: &str, shape: &[usize], v: &[i64]) {
        self.put(name, "int64", shape, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }

    pub fn i32(&mut self, name: &str, shape: &[usize], v: &[i32]) {
        self.put(name, "int32", shape, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }

    pub fn bools(&mut self, name: &str, shape: &[usize], v: &[bool]) {
        self.put(name, "bool", shape, v.iter().map(|&b| u8::from(b)).collect());
    }

    pub fn finish(self) {
        fs::write(self.dir.join("meta.json"), format!("{{{}}}", self.meta.join(", "))).expect("meta.json");
    }
}
