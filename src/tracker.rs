//! HTTP tracker announce (BEP 3, compact peer lists from BEP 23).

use crate::bencode::{self, Value};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

pub fn urlencode(b: &[u8]) -> String {
    b.iter()
        .map(|&c| match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (c as char).to_string(),
            _ => format!("%{c:02X}"),
        })
        .collect()
}

#[derive(Debug)]
pub struct Announce {
    pub interval: u64,
    pub peers: Vec<SocketAddr>,
}

pub fn parse_response(body: &[u8]) -> Result<Announce, String> {
    let v = bencode::decode(body).map_err(|e| e.to_string())?;
    if let Some(reason) = v.get("failure reason").and_then(Value::as_str) {
        return Err(format!("tracker: {reason}"));
    }
    let interval = v.get("interval").and_then(Value::as_int).unwrap_or(1800) as u64;
    let peers = match v.get("peers") {
        Some(Value::Bytes(b)) => b
            .chunks_exact(6)
            .map(|c| SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(c[0], c[1], c[2], c[3]), u16::from_be_bytes([c[4], c[5]]))))
            .collect(),
        Some(Value::List(l)) => l
            .iter()
            .filter_map(|p| {
                let ip = p.get("ip")?.as_str()?;
                let port = p.get("port")?.as_int()? as u16;
                format!("{ip}:{port}").parse().ok()
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(Announce { interval, peers })
}

pub fn announce(url: &str, info_hash: &[u8; 20], peer_id: &[u8; 20], port: u16, left: u64) -> Result<Announce, String> {
    let sep = if url.contains('?') { '&' } else { '?' };
    let full = format!(
        "{url}{sep}info_hash={}&peer_id={}&port={port}&uploaded=0&downloaded=0&left={left}&compact=1&event=started",
        urlencode(info_hash),
        urlencode(peer_id)
    );
    let resp = ureq::get(&full).timeout(std::time::Duration::from_secs(15)).call().map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut body).map_err(|e| e.to_string())?;
    parse_response(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_peers() {
        let mut body = b"d8:intervali900e5:peers12:".to_vec();
        body.extend_from_slice(&[127, 0, 0, 1, 0x1A, 0xE1, 10, 0, 0, 2, 0x1A, 0xE2]);
        body.push(b'e');
        let a = parse_response(&body).unwrap();
        assert_eq!(a.interval, 900);
        assert_eq!(a.peers, vec!["127.0.0.1:6881".parse().unwrap(), "10.0.0.2:6882".parse().unwrap()]);
    }

    #[test]
    fn surfaces_failure_reason() {
        assert_eq!(parse_response(b"d14:failure reason7:unknowne").unwrap_err(), "tracker: unknown");
    }

    #[test]
    fn encodes_bytes() {
        assert_eq!(urlencode(&[0x12, b'a', 0xff, b' ']), "%12a%FF%20");
    }
}
