//! Minimal GGUF header reader: the key/value metadata at the start of a .gguf file.
//! Only scalar values are kept; arrays (tokenizer vocabularies etc.) are skipped.
//! Format: https://github.com/ggml-org/ggml/blob/master/docs/gguf.md

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

pub struct Header {
    pub kv: HashMap<String, Val>,
}

impl Header {
    pub fn arch(&self) -> Option<&str> {
        self.str("general.architecture")
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.kv.get(key)? {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn int(&self, key: &str) -> Option<i64> {
        match self.kv.get(key)? {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }
    /// `<arch>.<key>`, e.g. arch_int("context_length") -> bert.context_length.
    pub fn arch_int(&self, key: &str) -> Option<i64> {
        self.int(&format!("{}.{key}", self.arch()?))
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn bytes<const N: usize>(r: &mut impl Read) -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}
fn u32_(r: &mut impl Read) -> io::Result<u32> {
    Ok(u32::from_le_bytes(bytes(r)?))
}
fn u64_(r: &mut impl Read) -> io::Result<u64> {
    Ok(u64::from_le_bytes(bytes(r)?))
}

fn string(r: &mut impl Read) -> io::Result<String> {
    let len = u64_(r)?;
    if len > 1 << 24 {
        return Err(bad("string too long"));
    }
    let mut b = vec![0u8; len as usize];
    r.read_exact(&mut b)?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

/// Reads one value of type `t`; arrays are read through and returned as None.
fn value(r: &mut impl Read, t: u32) -> io::Result<Option<Val>> {
    Ok(Some(match t {
        0 => Val::Int(u8::from_le_bytes(bytes(r)?) as i64),
        1 => Val::Int(i8::from_le_bytes(bytes(r)?) as i64),
        2 => Val::Int(u16::from_le_bytes(bytes(r)?) as i64),
        3 => Val::Int(i16::from_le_bytes(bytes(r)?) as i64),
        4 => Val::Int(u32::from_le_bytes(bytes(r)?) as i64),
        5 => Val::Int(i32::from_le_bytes(bytes(r)?) as i64),
        6 => Val::Float(f32::from_le_bytes(bytes(r)?) as f64),
        7 => Val::Bool(bytes::<1>(r)?[0] != 0),
        8 => Val::Str(string(r)?),
        9 => {
            let (inner, n) = (u32_(r)?, u64_(r)?);
            for _ in 0..n {
                value(r, inner)?;
            }
            return Ok(None);
        }
        10 => Val::Int(u64_(r)? as i64),
        11 => Val::Int(i64::from_le_bytes(bytes(r)?)),
        12 => Val::Float(f64::from_le_bytes(bytes(r)?)),
        _ => return Err(bad("unknown GGUF value type")),
    }))
}

pub fn parse(r: &mut impl Read) -> io::Result<Header> {
    if &bytes::<4>(r)? != b"GGUF" {
        return Err(bad("not a GGUF file"));
    }
    if u32_(r)? < 2 {
        return Err(bad("GGUF version 1 is not supported"));
    }
    let _tensors = u64_(r)?;
    let n = u64_(r)?;
    let mut kv = HashMap::new();
    for _ in 0..n {
        let key = string(r)?;
        let t = u32_(r)?;
        if let Some(v) = value(r, t)? {
            kv.insert(key, v);
        }
    }
    Ok(Header { kv })
}

pub fn read_header(path: &Path) -> io::Result<Header> {
    parse(&mut BufReader::new(File::open(path)?))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn s(out: &mut Vec<u8>, v: &str) {
        out.extend((v.len() as u64).to_le_bytes());
        out.extend(v.as_bytes());
    }

    /// A tiny GGUF header like an embedding model's (bert, CLS pooling, 512 ctx, 384 dims).
    pub fn fake_embed_header() -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes()); // tensors
        b.extend(5u64.to_le_bytes()); // kv pairs
        s(&mut b, "general.architecture");
        b.extend(8u32.to_le_bytes());
        s(&mut b, "bert");
        s(&mut b, "bert.context_length");
        b.extend(4u32.to_le_bytes());
        b.extend(512u32.to_le_bytes());
        s(&mut b, "tokenizer.ggml.tokens"); // array of strings, skipped
        b.extend(9u32.to_le_bytes());
        b.extend(8u32.to_le_bytes());
        b.extend(2u64.to_le_bytes());
        s(&mut b, "[CLS]");
        s(&mut b, "hello");
        s(&mut b, "bert.embedding_length");
        b.extend(4u32.to_le_bytes());
        b.extend(384u32.to_le_bytes());
        s(&mut b, "bert.pooling_type");
        b.extend(4u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        b
    }

    #[test]
    fn reads_fake_header() {
        let h = parse(&mut fake_embed_header().as_slice()).unwrap();
        assert_eq!(h.arch(), Some("bert"));
        assert_eq!(h.arch_int("context_length"), Some(512));
        assert_eq!(h.arch_int("embedding_length"), Some(384));
        assert_eq!(h.arch_int("pooling_type"), Some(2));
        assert!(!h.kv.contains_key("tokenizer.ggml.tokens"));
        assert!(parse(&mut &b"GGML...."[..]).is_err());
    }

    /// Reads every .gguf on the drive, if any (run locally; CI has no models).
    #[test]
    fn reads_real_files_if_present() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.system");
        let files = [root.read_dir(), root.join("embed").read_dir()];
        for entry in files.into_iter().flatten().flatten().flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "gguf") {
                let h = read_header(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                assert!(h.arch().is_some(), "{}", p.display());
                assert!(h.arch_int("context_length").is_some(), "{}", p.display());            }
        }
    }
}
