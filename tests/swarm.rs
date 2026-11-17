use bittorrent_client::download::{download, Progress};
use bittorrent_client::metainfo::Torrent;
use bittorrent_client::seed::Seeder;
use std::net::TcpListener;
use std::sync::Arc;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("bt-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn downloads_from_two_local_seeders() {
    let seed_dir = tmp("seed");
    let data: Vec<u8> = (0..1_234_567u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    std::fs::write(seed_dir.join("payload.bin"), &data).unwrap();
    let raw = Torrent::create("payload.bin", &data, 64 * 1024, None);
    let torrent = Torrent::from_bytes(&raw).unwrap();

    let mut peers = Vec::new();
    for _ in 0..2 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        peers.push(l.local_addr().unwrap());
        let s = Seeder::new(torrent.clone(), &seed_dir).unwrap();
        std::thread::spawn(move || s.serve(l));
    }

    let out = tmp("leech");
    let progress = Arc::new(Progress::default());
    download(torrent.clone(), peers.clone(), &out, progress.clone()).unwrap();
    assert_eq!(std::fs::read(out.join("payload.bin")).unwrap(), data);

    // second run resumes: everything verifies from disk, nothing is fetched
    let p2 = Arc::new(Progress::default());
    download(torrent, peers, &out, p2.clone()).unwrap();
    assert_eq!(p2.bytes.load(std::sync::atomic::Ordering::SeqCst), 0);

    std::fs::remove_dir_all(seed_dir).ok();
    std::fs::remove_dir_all(out).ok();
}
