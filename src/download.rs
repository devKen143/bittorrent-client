//! Multi-peer piece downloader with request pipelining, hash checks and resume.

use crate::metainfo::{sha1, Torrent};
use crate::peer::{self, has_piece, read_msg, set_piece, write_msg, Message, BLOCK};
use crate::storage::Storage;
use std::collections::VecDeque;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PIPELINE: usize = 8;
const MAX_PEERS: usize = 30;

#[derive(Default)]
pub struct Progress {
    pub pieces_done: AtomicUsize,
    pub pieces_total: AtomicUsize,
    pub bytes: AtomicU64,
    pub active_peers: AtomicUsize,
    pub hash_failures: AtomicUsize,
}

struct Shared {
    torrent: Torrent,
    storage: Storage,
    queue: Mutex<VecDeque<usize>>,
    have: Mutex<Vec<u8>>,
    progress: Arc<Progress>,
    peer_id: [u8; 20],
}

pub fn random_peer_id() -> [u8; 20] {
    use std::hash::{BuildHasher, Hasher};
    let mut id = *b"-RS0100-000000000000";
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(std::process::id() as u64);
    let mut x = h.finish();
    for b in id[8..].iter_mut() {
        *b = b'0' + (x % 10) as u8;
        x /= 10;
        if x == 0 {
            x = 0x9E37_79B9_7F4A_7C15;
        }
    }
    id
}

/// Verifies whatever is already on disk so interrupted downloads resume.
fn existing_pieces(t: &Torrent, s: &Storage) -> Vec<usize> {
    (0..t.pieces.len())
        .filter(|&i| s.read(i as u64 * t.piece_length, t.piece_size(i) as usize).map(|d| sha1(&d) == t.pieces[i]).unwrap_or(false))
        .collect()
}

pub fn download(
    torrent: Torrent,
    peers: Vec<SocketAddr>,
    out_dir: &Path,
    progress: Arc<Progress>,
) -> Result<(), String> {
    let storage = Storage::create(out_dir, &torrent.files).map_err(|e| e.to_string())?;
    let n = torrent.pieces.len();
    let mut have = vec![0u8; n.div_ceil(8)];
    let done = existing_pieces(&torrent, &storage);
    for &i in &done {
        set_piece(&mut have, i);
    }
    progress.pieces_total.store(n, Ordering::SeqCst);
    progress.pieces_done.store(done.len(), Ordering::SeqCst);
    let queue: VecDeque<usize> = (0..n).filter(|i| !has_piece(&have, *i)).collect();
    if queue.is_empty() {
        return Ok(());
    }
    if peers.is_empty() {
        return Err("no peers".into());
    }
    let shared = Arc::new(Shared { torrent, storage, queue: Mutex::new(queue), have: Mutex::new(have), progress, peer_id: random_peer_id() });

    let handles: Vec<_> = peers
        .into_iter()
        .take(MAX_PEERS)
        .map(|addr| {
            let s = shared.clone();
            std::thread::spawn(move || {
                s.progress.active_peers.fetch_add(1, Ordering::SeqCst);
                let r = worker(addr, &s);
                s.progress.active_peers.fetch_sub(1, Ordering::SeqCst);
                r
            })
        })
        .collect();
    let mut errors = Vec::new();
    for h in handles {
        if let Ok(Err(e)) = h.join() {
            errors.push(e);
        }
    }
    shared.storage.flush().map_err(|e| e.to_string())?;
    let finished = shared.progress.pieces_done.load(Ordering::SeqCst);
    if finished == n {
        Ok(())
    } else {
        Err(format!("incomplete: {finished}/{n} pieces; peer errors: {}", errors.join("; ")))
    }
}

fn next_piece(s: &Shared, bitfield: &[u8]) -> Option<usize> {
    let mut q = s.queue.lock().unwrap();
    let pos = q.iter().position(|&i| has_piece(bitfield, i))?;
    q.remove(pos)
}

fn worker(addr: SocketAddr, s: &Shared) -> Result<(), String> {
    let e = |err: std::io::Error| format!("{addr}: {err}");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(e)?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(e)?;
    stream.set_nodelay(true).ok();
    peer::handshake(&mut stream, &s.torrent.info_hash, &s.peer_id).map_err(e)?;
    write_msg(&mut stream, &Message::Interested).map_err(e)?;

    let n = s.torrent.pieces.len();
    let mut bitfield = vec![0u8; n.div_ceil(8)];
    let mut choked = true;

    loop {
        // wait until unchoked
        while choked {
            match read_msg(&mut stream).map_err(e)? {
                Message::Unchoke => choked = false,
                Message::Bitfield(b) => bitfield = b,
                Message::Have(i) => set_piece(&mut bitfield, i as usize),
                _ => {}
            }
        }
        let Some(index) = next_piece(s, &bitfield) else {
            if s.queue.lock().unwrap().is_empty() {
                return Ok(());
            }
            return Ok(()); // this peer has nothing we still need
        };
        match fetch_piece(&mut stream, s, index, &mut bitfield, &mut choked) {
            Ok(Some(data)) if sha1(&data) == s.torrent.pieces[index] => {
                s.storage.write(index as u64 * s.torrent.piece_length, &data).map_err(e)?;
                set_piece(&mut s.have.lock().unwrap(), index);
                s.progress.pieces_done.fetch_add(1, Ordering::SeqCst);
                s.progress.bytes.fetch_add(data.len() as u64, Ordering::SeqCst);
                write_msg(&mut stream, &Message::Have(index as u32)).ok();
            }
            Ok(Some(_)) => {
                s.progress.hash_failures.fetch_add(1, Ordering::SeqCst);
                s.queue.lock().unwrap().push_back(index);
                return Err(format!("{addr}: sent corrupt data for piece {index}"));
            }
            Ok(None) => s.queue.lock().unwrap().push_front(index), // choked mid-piece
            Err(err) => {
                s.queue.lock().unwrap().push_front(index);
                return Err(e(err));
            }
        }
    }
}

/// Downloads one piece; returns None if the peer choked us part-way through.
fn fetch_piece(stream: &mut TcpStream, s: &Shared, index: usize, bitfield: &mut [u8], choked: &mut bool) -> std::io::Result<Option<Vec<u8>>> {
    let size = s.torrent.piece_size(index) as u32;
    let mut buf = vec![0u8; size as usize];
    let blocks = size.div_ceil(BLOCK);
    let (mut requested, mut received, mut in_flight) = (0u32, 0u32, 0usize);
    while received < blocks {
        while in_flight < PIPELINE && requested < blocks {
            let begin = requested * BLOCK;
            let length = BLOCK.min(size - begin);
            write_msg(stream, &Message::Request { index: index as u32, begin, length })?;
            requested += 1;
            in_flight += 1;
        }
        match read_msg(stream)? {
            Message::Piece { index: i, begin, block } if i as usize == index => {
                let end = begin as usize + block.len();
                if end > buf.len() {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "block out of range"));
                }
                buf[begin as usize..end].copy_from_slice(&block);
                received += 1;
                in_flight -= 1;
            }
            Message::Choke => {
                *choked = true;
                return Ok(None);
            }
            Message::Have(i) => set_piece(bitfield, i as usize),
            _ => {}
        }
    }
    Ok(Some(buf))
}
