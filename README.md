# bittorrent-client

A BitTorrent client **and** seeder written from the spec in Rust (BEP 3, BEP 23).

## Features
- Strict bencode decoder and canonical encoder
- `.torrent` parsing, info-hash, magnet links, multi-file torrents with path-traversal protection
- HTTP tracker announce (compact + dictionary peer lists)
- Peer wire protocol: handshake, bitfield, have, choke/unchoke, interested, request/piece with 8-deep pipelining
- Up to 30 peers in parallel, SHA-1 verification of every piece, corrupt peers dropped
- Resume: pieces already on disk are verified and skipped
- `bt seed` serves a torrent so you can build a local swarm

## Usage
```bash
cargo build --release
bt info file.torrent
bt download file.torrent -o downloads/            # uses the tracker
bt create video.mp4 -o video.torrent
bt seed video.torrent --dir . --port 6881
bt download video.torrent -o copy/ --peer 127.0.0.1:6881
```

## Tests
`cargo test` — includes an end-to-end test that downloads a 1.2 MB payload from two local seeders and then verifies resume.

## Web
`web/` is a React (Vite) site that inspects and creates `.torrent` files in the browser.

## License
MIT
