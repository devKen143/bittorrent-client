export default {
  name: 'bittorrent-client',
  stack: 'Rust · threads · TCP',
  tagline: 'A BitTorrent client and seeder built from the spec: bencode, metainfo, HTTP trackers and the peer wire protocol.',
  repo: 'https://github.com/devKen143/bittorrent-client',
  description:
    'bt reads .torrent files, asks the tracker for peers, opens up to 30 peer connections in parallel, pipelines 16 KiB block requests, verifies every piece with SHA-1 and writes it into place across one or many files. It can also seed, so two machines (or one test) can form a swarm.',
  features: [
    'Strict bencode decoder (rejects leading zeros, trailing data, deep nesting) and canonical encoder',
    'Info-hash computation, magnet links, multi-file torrents with path-traversal protection',
    'HTTP tracker announce with compact (BEP 23) and dictionary peer lists',
    'Peer wire protocol: handshake, bitfield, have, choke/unchoke, request/piece pipelining',
    'Resume: pieces already on disk are hash-checked and skipped',
    'Seeder that validates every request before reading from disk',
  ],
  demoNote:
    'Drop any .torrent file to decode it in the browser (the info hash is computed with SHA-1 just like the Rust code), or drop any file to create a .torrent for it. The swarm view animates how pieces arrive from several peers.',
  architecture: [
    { title: 'bencode.rs', text: 'Values are Int, Bytes, List and Dict (BTreeMap, so re-encoding is canonical — that is what makes the info hash stable).' },
    { title: 'download.rs', text: 'A shared queue of missing piece indices. Each peer thread takes the first piece that peer has, keeps 8 requests in flight, and returns the piece to the queue on choke or error.' },
    { title: 'storage.rs', text: 'The torrent is one flat byte range. Reads and writes are split across file boundaries so multi-file torrents just work.' },
    { title: 'seed.rs', text: 'Accepts handshakes, sends a full bitfield with the spare bits cleared, unchokes on interest and serves bounds-checked block requests.' },
  ],
  usage: `git clone https://github.com/devKen143/bittorrent-client && cd bittorrent-client
cargo build --release

./target/release/bt info ubuntu.iso.torrent
./target/release/bt download ubuntu.iso.torrent -o downloads/

# make your own swarm
./target/release/bt create video.mp4 -o video.torrent
./target/release/bt seed video.torrent --dir . --port 6881
./target/release/bt download video.torrent -o copy/ --peer 127.0.0.1:6881`,
}
