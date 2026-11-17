import { useEffect, useRef, useState } from 'react'

// ---- bencode (mirror of src/bencode.rs) ----
function bdecode(buf) {
  let pos = 0
  const val = () => {
    const c = buf[pos]
    if (c === 0x69) { const end = buf.indexOf(0x65, pos); const n = Number(new TextDecoder().decode(buf.slice(pos + 1, end))); pos = end + 1; return n }
    if (c === 0x6c) { pos++; const l = []; while (buf[pos] !== 0x65) l.push(val()); pos++; return l }
    if (c === 0x64) {
      pos++; const d = {}; const raw = {}
      while (buf[pos] !== 0x65) { const k = new TextDecoder().decode(val()); const s = pos; d[k] = val(); raw[k] = [s, pos] }
      pos++; Object.defineProperty(d, '__raw', { value: raw }); return d
    }
    if (c >= 0x30 && c <= 0x39) { const colon = buf.indexOf(0x3a, pos); const len = Number(new TextDecoder().decode(buf.slice(pos, colon))); pos = colon + 1; const b = buf.slice(pos, pos + len); pos += len; return b }
    throw new Error(`bad bencode at byte ${pos}`)
  }
  return val()
}
const enc = new TextEncoder()
function bencode(v) {
  if (typeof v === 'number') return enc.encode(`i${v}e`)
  if (typeof v === 'string') v = enc.encode(v)
  if (v instanceof Uint8Array) return concat([enc.encode(`${v.length}:`), v])
  if (Array.isArray(v)) return concat([enc.encode('l'), ...v.map(bencode), enc.encode('e')])
  return concat([enc.encode('d'), ...Object.keys(v).sort().flatMap((k) => [bencode(k), bencode(v[k])]), enc.encode('e')])
}
function concat(parts) { const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0)); let o = 0; for (const p of parts) { out.set(p, o); o += p.length } return out }
const hex = (b) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, '0')).join('')
const str = (b) => (b instanceof Uint8Array ? new TextDecoder().decode(b) : String(b))
const size = (n) => n > 1e9 ? `${(n / 1e9).toFixed(2)} GB` : n > 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n > 1e3 ? `${(n / 1e3).toFixed(1)} KB` : `${n} B`

async function inspect(buf) {
  const root = bdecode(buf)
  const [s, e] = root.__raw.info
  const infoHash = hex(await crypto.subtle.digest('SHA-1', buf.slice(s, e)))
  const info = root.info
  const files = info.files ? info.files.map((f) => ({ path: f.path.map(str).join('/'), length: f.length })) : [{ path: str(info.name), length: info.length }]
  return { name: str(info.name), infoHash, announce: root.announce ? str(root.announce) : '', pieceLength: info['piece length'], pieces: info.pieces.length / 20, files, total: files.reduce((n, f) => n + f.length, 0) }
}

async function create(file) {
  const data = new Uint8Array(await file.arrayBuffer())
  const pl = data.length > 64 * 1024 * 1024 ? 1 << 20 : 256 * 1024
  const hashes = []
  for (let o = 0; o < Math.max(data.length, 1); o += pl) hashes.push(new Uint8Array(await crypto.subtle.digest('SHA-1', data.slice(o, o + pl))))
  return bencode({ 'created by': 'bittorrent-client demo', info: { length: data.length, name: file.name, 'piece length': pl, pieces: concat(hashes) } })
}

function Swarm() {
  const N = 96
  const PEERS = [{ name: 'peer A', color: '#c2410c' }, { name: 'peer B', color: '#2563eb' }, { name: 'peer C', color: '#15803d' }, { name: 'peer D', color: '#9333ea' }]
  const [pieces, setPieces] = useState(() => Array(N).fill(null))
  const [running, setRunning] = useState(false)
  const timer = useRef(null)
  useEffect(() => {
    if (!running) return
    timer.current = setInterval(() => {
      setPieces((cur) => {
        const missing = cur.map((p, i) => (p === null ? i : -1)).filter((i) => i >= 0)
        if (!missing.length) { setRunning(false); return cur }
        const next = [...cur]
        PEERS.forEach((_, pi) => { if (Math.random() < 0.8 && missing.length) { const k = Math.floor(Math.random() * missing.length); next[missing.splice(k, 1)[0]] = pi } })
        return next
      })
    }, 90)
    return () => clearInterval(timer.current)
  }, [running])
  const done = pieces.filter((p) => p !== null).length
  return (
    <div>
      <div className="row">
        <button className="primary" onClick={() => { if (done === N) setPieces(Array(N).fill(null)); setRunning(!running) }}>{running ? 'Pause' : done === N ? 'Restart' : 'Start download'}</button>
        <span className="pill">{done}/{N} pieces verified</span>
        {PEERS.map((p, i) => <span key={p.name} className="pill" style={{ color: p.color }}>■ {p.name}: {pieces.filter((x) => x === i).length}</span>)}
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(16px, 1fr))', gap: 3, marginTop: 8 }}>
        {pieces.map((p, i) => <div key={i} title={`piece ${i}`} style={{ aspectRatio: '1', borderRadius: 3, background: p === null ? 'var(--surface-2)' : PEERS[p].color }} />)}
      </div>
    </div>
  )
}

export default function Demo() {
  const [tab, setTab] = useState('inspect')
  const [info, setInfo] = useState(null)
  const [err, setErr] = useState('')
  const [created, setCreated] = useState(null)

  async function onTorrent(f) {
    setErr(''); setInfo(null)
    try { setInfo(await inspect(new Uint8Array(await f.arrayBuffer()))) } catch (e) { setErr(e.message) }
  }
  async function onCreate(f) {
    setErr('')
    const bytes = await create(f)
    setCreated({ url: URL.createObjectURL(new Blob([bytes], { type: 'application/x-bittorrent' })), name: `${f.name}.torrent`, info: await inspect(bytes) })
  }

  return (
    <div>
      <div className="row">
        {[['inspect', 'Inspect .torrent'], ['create', 'Create .torrent'], ['swarm', 'Swarm view']].map(([k, l]) => (
          <button key={k} className={tab === k ? 'primary' : ''} onClick={() => setTab(k)}>{l}</button>
        ))}
      </div>
      {tab === 'inspect' && (
        <div>
          <input type="file" accept=".torrent" onChange={(e) => e.target.files[0] && onTorrent(e.target.files[0])} aria-label="torrent file" />
          {err && <p className="pill err">{err}</p>}
          {info && <TorrentInfo info={info} />}
        </div>
      )}
      {tab === 'create' && (
        <div>
          <p className="muted" style={{ fontSize: 14 }}>Pick any file. Pieces are hashed locally — nothing is uploaded.</p>
          <input type="file" onChange={(e) => e.target.files[0] && onCreate(e.target.files[0])} aria-label="file to share" />
          {created && (<div><p><a className="btn primary" href={created.url} download={created.name}>Download {created.name}</a></p><TorrentInfo info={created.info} /></div>)}
        </div>
      )}
      {tab === 'swarm' && <Swarm />}
    </div>
  )
}

function TorrentInfo({ info }) {
  return (
    <table style={{ marginTop: 12 }}>
      <tbody>
        <tr><th>Name</th><td>{info.name}</td></tr>
        <tr><th>Info hash</th><td><code style={{ wordBreak: 'break-all' }}>{info.infoHash}</code></td></tr>
        <tr><th>Size</th><td>{size(info.total)} in {info.files.length} file(s)</td></tr>
        <tr><th>Pieces</th><td>{info.pieces} × {size(info.pieceLength)}</td></tr>
        <tr><th>Tracker</th><td>{info.announce || '—'}</td></tr>
        <tr><th>Magnet</th><td><code style={{ wordBreak: 'break-all', fontSize: 12 }}>magnet:?xt=urn:btih:{info.infoHash}&dn={encodeURIComponent(info.name)}</code></td></tr>
        {info.files.slice(0, 20).map((f) => <tr key={f.path}><td className="muted">{size(f.length)}</td><td>{f.path}</td></tr>)}
      </tbody>
    </table>
  )
}
