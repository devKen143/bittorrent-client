//! Maps the torrent's flat byte space onto one or more files on disk.

use crate::metainfo::FileEntry;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

pub struct Storage {
    files: Mutex<Vec<(File, u64, u64)>>, // (handle, global start offset, length)
}

impl Storage {
    /// Opens (creating and sizing if needed) every file under `dir`.
    pub fn create(dir: &Path, entries: &[FileEntry]) -> io::Result<Self> {
        Self::open_inner(dir, entries, true)
    }

    /// Opens existing files read-only (used when seeding).
    pub fn open(dir: &Path, entries: &[FileEntry]) -> io::Result<Self> {
        Self::open_inner(dir, entries, false)
    }

    fn open_inner(dir: &Path, entries: &[FileEntry], write: bool) -> io::Result<Self> {
        let mut files = Vec::new();
        let mut offset = 0;
        for e in entries {
            let path = dir.join(&e.path);
            let f = if write {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
                if f.metadata()?.len() != e.length {
                    f.set_len(e.length)?;
                }
                f
            } else {
                let f = File::open(&path)?;
                if f.metadata()?.len() != e.length {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} has the wrong size", path.display())));
                }
                f
            };
            files.push((f, offset, e.length));
            offset += e.length;
        }
        Ok(Storage { files: Mutex::new(files) })
    }

    fn for_each_span(&self, offset: u64, len: u64, mut f: impl FnMut(&mut File, u64, usize, usize) -> io::Result<()>) -> io::Result<()> {
        let mut files = self.files.lock().unwrap();
        let end = offset + len;
        for (file, start, flen) in files.iter_mut() {
            let fend = *start + *flen;
            if fend <= offset || *start >= end {
                continue;
            }
            let from = offset.max(*start);
            let to = end.min(fend);
            f(file, from - *start, (from - offset) as usize, (to - from) as usize)?;
        }
        Ok(())
    }

    pub fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.for_each_span(offset, len as u64, |file, file_off, buf_off, n| {
            file.seek(SeekFrom::Start(file_off))?;
            file.read_exact(&mut buf[buf_off..buf_off + n])
        })?;
        Ok(buf)
    }

    pub fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.for_each_span(offset, data.len() as u64, |file, file_off, buf_off, n| {
            file.seek(SeekFrom::Start(file_off))?;
            file.write_all(&data[buf_off..buf_off + n])
        })
    }

    pub fn flush(&self) -> io::Result<()> {
        for (f, _, _) in self.files.lock().unwrap().iter_mut() {
            f.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn spans_multiple_files() {
        let dir = std::env::temp_dir().join(format!("bt-storage-{}", std::process::id()));
        let entries = vec![
            FileEntry { path: PathBuf::from("t/a"), length: 5 },
            FileEntry { path: PathBuf::from("t/b"), length: 3 },
            FileEntry { path: PathBuf::from("t/c"), length: 4 },
        ];
        let s = Storage::create(&dir, &entries).unwrap();
        s.write(3, b"HELLOWOR").unwrap();
        assert_eq!(s.read(0, 12).unwrap(), b"\0\0\0HELLOWOR\0");
        assert_eq!(std::fs::read(dir.join("t/b")).unwrap(), b"LLO");
        drop(s);
        std::fs::remove_dir_all(dir).ok();
    }
}
