use bittorrent_client::download::{download, random_peer_id, Progress};
use bittorrent_client::metainfo::{hex, Torrent};
use bittorrent_client::seed::Seeder;
use bittorrent_client::tracker;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = "\
bt — a BitTorrent client

USAGE:
  bt info <file.torrent>
  bt create <file> -o <out.torrent> [--piece-length 262144] [--announce URL]
  bt download <file.torrent> [-o <dir>] [--peer host:port ...]
  bt seed <file.torrent> --dir <data dir> [--port 6881]
";

type R = Result<(), Box<dyn std::error::Error>>;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("info") => info(&args),
        Some("create") => create(&args),
        Some("download") => get(&args),
        Some("seed") => seed(&args),
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2)
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn load(args: &[String]) -> Result<Torrent, Box<dyn std::error::Error>> {
    Ok(Torrent::from_file(Path::new(args.get(1).ok_or("missing torrent path")?))?)
}

fn info(args: &[String]) -> R {
    let t = load(args)?;
    println!("name:         {}", t.name);
    println!("info hash:    {}", hex(&t.info_hash));
    println!("announce:     {}", t.announce.as_deref().unwrap_or("-"));
    println!("size:         {} bytes", t.total_length());
    println!("pieces:       {} x {} bytes", t.pieces.len(), t.piece_length);
    println!("magnet:       {}", t.magnet());
    for f in &t.files {
        println!("  {:>12}  {}", f.length, f.path.display());
    }
    Ok(())
}

fn create(args: &[String]) -> R {
    let path = PathBuf::from(args.get(1).ok_or("missing file")?);
    let out = flag(args, "-o").ok_or("missing -o")?;
    let pl: u64 = flag(args, "--piece-length").map(|s| s.parse()).transpose()?.unwrap_or(256 * 1024);
    let data = std::fs::read(&path)?;
    let name = path.file_name().ok_or("bad file name")?.to_string_lossy().to_string();
    std::fs::write(&out, Torrent::create(&name, &data, pl, flag(args, "--announce").as_deref()))?;
    println!("wrote {out}");
    Ok(())
}

fn get(args: &[String]) -> R {
    let t = load(args)?;
    let out = PathBuf::from(flag(args, "-o").unwrap_or_else(|| ".".into()));
    let mut peers: Vec<SocketAddr> =
        args.windows(2).filter(|w| w[0] == "--peer").map(|w| w[1].parse()).collect::<Result<_, _>>()?;
    if peers.is_empty() {
        let url = t.announce.clone().ok_or("torrent has no tracker; pass --peer")?;
        let a = tracker::announce(&url, &t.info_hash, &random_peer_id(), 6881, t.total_length())?;
        println!("tracker returned {} peers", a.peers.len());
        peers = a.peers;
    }
    let progress = Arc::new(Progress::default());
    let p = progress.clone();
    let total_bytes = t.total_length();
    let started = Instant::now();
    let reporter = std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let (d, n) = (p.pieces_done.load(Ordering::SeqCst), p.pieces_total.load(Ordering::SeqCst));
        let mbps = p.bytes.load(Ordering::SeqCst) as f64 / started.elapsed().as_secs_f64() / 1e6;
        eprint!("\r{d}/{n} pieces  {:.1} MB/s  {} peers   ", mbps, p.active_peers.load(Ordering::SeqCst));
        if n > 0 && d >= n {
            break;
        }
    });
    let result = download(t, peers, &out, progress.clone());
    progress.pieces_done.store(progress.pieces_total.load(Ordering::SeqCst), Ordering::SeqCst);
    reporter.join().ok();
    eprintln!();
    result?;
    println!("done: {total_bytes} bytes in {:.1?} -> {}", started.elapsed(), out.display());
    Ok(())
}

fn seed(args: &[String]) -> R {
    let t = load(args)?;
    let dir = PathBuf::from(flag(args, "--dir").unwrap_or_else(|| ".".into()));
    let port = flag(args, "--port").unwrap_or_else(|| "6881".into());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
    println!("seeding {} ({}) on port {port}", t.name, hex(&t.info_hash));
    Seeder::new(t, &dir)?.serve(listener);
    Ok(())
}
