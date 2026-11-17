//! A minimal seeder: answers handshakes, advertises every piece, serves block requests.

use crate::metainfo::Torrent;
use crate::peer::{read_handshake, read_msg, send_handshake, write_msg, Message};
use crate::storage::Storage;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct Seeder {
    torrent: Torrent,
    storage: Storage,
    peer_id: [u8; 20],
    pub uploaded: AtomicU64,
}

impl Seeder {
    pub fn new(torrent: Torrent, data_dir: &std::path::Path) -> std::io::Result<Arc<Self>> {
        let storage = Storage::open(data_dir, &torrent.files)?;
        Ok(Arc::new(Seeder { torrent, storage, peer_id: *b"-RS0100-seed00000000", uploaded: AtomicU64::new(0) }))
    }

    pub fn serve(self: Arc<Self>, listener: TcpListener) {
        for conn in listener.incoming().flatten() {
            let me = self.clone();
            std::thread::spawn(move || {
                let _ = me.handle(conn);
            });
        }
    }

    fn handle(&self, mut s: TcpStream) -> std::io::Result<()> {
        s.set_nodelay(true).ok();
        read_handshake(&mut s, &self.torrent.info_hash)?;
        send_handshake(&mut s, &self.torrent.info_hash, &self.peer_id)?;
        let n = self.torrent.pieces.len();
        let mut bf = vec![0xFFu8; n.div_ceil(8)];
        if n % 8 != 0 {
            *bf.last_mut().unwrap() = 0xFF << (8 - n % 8); // spare bits must be zero
        }
        write_msg(&mut s, &Message::Bitfield(bf))?;
        loop {
            match read_msg(&mut s)? {
                Message::Interested => write_msg(&mut s, &Message::Unchoke)?,
                Message::Request { index, begin, length } => {
                    let i = index as usize;
                    if i >= n || length > 128 * 1024 || begin as u64 + length as u64 > self.torrent.piece_size(i) {
                        return Ok(()); // protocol violation: drop the peer
                    }
                    let off = i as u64 * self.torrent.piece_length + begin as u64;
                    let block = self.storage.read(off, length as usize)?;
                    write_msg(&mut s, &Message::Piece { index, begin, block })?;
                    self.uploaded.fetch_add(length as u64, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }
}
