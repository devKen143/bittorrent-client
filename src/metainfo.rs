//! .torrent files: parsing, info-hash and creation.

use crate::bencode::{self, bytes, dict, Value};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub length: u64,
}

#[derive(Debug, Clone)]
pub struct Torrent {
    pub announce: Option<String>,
    pub name: String,
    pub piece_length: u64,
    pub pieces: Vec<[u8; 20]>,
    pub files: Vec<FileEntry>,
    pub info_hash: [u8; 20],
}

pub fn sha1(data: &[u8]) -> [u8; 20] {
    Sha1::digest(data).into()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Torrent {
    pub fn total_length(&self) -> u64 {
        self.files.iter().map(|f| f.length).sum()
    }

    pub fn piece_size(&self, index: usize) -> u64 {
        let start = index as u64 * self.piece_length;
        (self.total_length() - start).min(self.piece_length)
    }

    pub fn magnet(&self) -> String {
        let mut m = format!("magnet:?xt=urn:btih:{}&dn={}", hex(&self.info_hash), self.name.replace(' ', "%20"));
        if let Some(a) = &self.announce {
            m.push_str("&tr=");
            m.push_str(&crate::tracker::urlencode(a.as_bytes()));
        }
        m
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let root = bencode::decode(data).map_err(|e| e.to_string())?;
        let info = root.get("info").ok_or("missing info dict")?;
        let info_hash = sha1(&info.encode());
        let name = info.get("name").and_then(Value::as_str).ok_or("missing name")?.to_string();
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err("unsafe torrent name".into());
        }
        let piece_length = info.get("piece length").and_then(Value::as_int).filter(|&n| n > 0).ok_or("bad piece length")? as u64;
        let raw = info.get("pieces").and_then(Value::as_bytes).ok_or("missing pieces")?;
        if raw.len() % 20 != 0 {
            return Err("pieces length not a multiple of 20".into());
        }
        let pieces = raw.chunks(20).map(|c| c.try_into().unwrap()).collect::<Vec<[u8; 20]>>();

        let files = if let Some(len) = info.get("length").and_then(Value::as_int) {
            vec![FileEntry { path: PathBuf::from(&name), length: len as u64 }]
        } else {
            let list = info.get("files").and_then(Value::as_list).ok_or("neither length nor files")?;
            let mut out = Vec::new();
            for f in list {
                let length = f.get("length").and_then(Value::as_int).ok_or("file without length")? as u64;
                let mut path = PathBuf::from(&name);
                for part in f.get("path").and_then(Value::as_list).ok_or("file without path")? {
                    let p = part.as_str().ok_or("non-utf8 path")?;
                    if p == ".." || p.contains('/') || p.contains('\\') || p.is_empty() {
                        return Err("unsafe path component".into());
                    }
                    path.push(p);
                }
                out.push(FileEntry { path, length });
            }
            out
        };
        let t = Torrent {
            announce: root.get("announce").and_then(Value::as_str).map(String::from),
            name,
            piece_length,
            pieces,
            files,
            info_hash,
        };
        let expected = t.total_length().div_ceil(piece_length) as usize;
        if expected != t.pieces.len() {
            return Err(format!("expected {expected} piece hashes, found {}", t.pieces.len()));
        }
        Ok(t)
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        Self::from_bytes(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
    }

    /// Builds a single-file torrent for `data` (used by `bt create` and the tests).
    pub fn create(name: &str, data: &[u8], piece_length: u64, announce: Option<&str>) -> Vec<u8> {
        let pieces: Vec<u8> = data.chunks(piece_length as usize).flat_map(sha1).collect();
        let info = dict([
            ("length", Value::Int(data.len() as i64)),
            ("name", bytes(name)),
            ("piece length", Value::Int(piece_length as i64)),
            ("pieces", Value::Bytes(pieces)),
        ]);
        let mut root = dict([("info", info), ("created by", bytes("bittorrent-client (rust)"))]);
        if let (Some(a), Value::Dict(d)) = (announce, &mut root) {
            d.insert(b"announce".to_vec(), bytes(a));
        }
        root.encode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_parse() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let raw = Torrent::create("data.bin", &data, 16384, Some("http://tracker.local/announce"));
        let t = Torrent::from_bytes(&raw).unwrap();
        assert_eq!(t.total_length(), 100_000);
        assert_eq!(t.pieces.len(), 7);
        assert_eq!(t.piece_size(6), 100_000 - 6 * 16384);
        assert_eq!(t.pieces[0], sha1(&data[..16384]));
        assert!(t.magnet().starts_with("magnet:?xt=urn:btih:"));
    }

    #[test]
    fn rejects_path_traversal() {
        let info = dict([
            ("name", bytes("x")),
            ("piece length", Value::Int(4)),
            ("pieces", Value::Bytes(vec![0; 20])),
            ("files", Value::List(vec![dict([("length", Value::Int(1)), ("path", Value::List(vec![bytes("..")]))])])),
        ]);
        let raw = dict([("info", info)]).encode();
        assert!(Torrent::from_bytes(&raw).unwrap_err().contains("unsafe"));
    }
}
