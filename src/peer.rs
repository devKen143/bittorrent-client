//! Peer wire protocol: handshake and length-prefixed messages.

use std::io::{self, Read, Write};
use std::net::TcpStream;

pub const BLOCK: u32 = 16 * 1024;
const PROTOCOL: &[u8] = b"BitTorrent protocol";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request { index: u32, begin: u32, length: u32 },
    Piece { index: u32, begin: u32, block: Vec<u8> },
    Cancel { index: u32, begin: u32, length: u32 },
    Unknown(u8),
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

pub fn handshake(stream: &mut TcpStream, info_hash: &[u8; 20], peer_id: &[u8; 20]) -> io::Result<[u8; 20]> {
    let mut out = Vec::with_capacity(68);
    out.push(PROTOCOL.len() as u8);
    out.extend_from_slice(PROTOCOL);
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(info_hash);
    out.extend_from_slice(peer_id);
    stream.write_all(&out)?;
    read_handshake(stream, info_hash)
}

/// Reads the remote handshake and checks the info hash. Returns the remote peer id.
pub fn read_handshake(stream: &mut TcpStream, info_hash: &[u8; 20]) -> io::Result<[u8; 20]> {
    let mut buf = [0u8; 68];
    stream.read_exact(&mut buf)?;
    if buf[0] as usize != PROTOCOL.len() || &buf[1..20] != PROTOCOL {
        return Err(bad("not a BitTorrent handshake"));
    }
    if &buf[28..48] != info_hash {
        return Err(bad("info hash mismatch"));
    }
    Ok(buf[48..68].try_into().unwrap())
}

pub fn send_handshake(stream: &mut TcpStream, info_hash: &[u8; 20], peer_id: &[u8; 20]) -> io::Result<()> {
    let mut out = vec![PROTOCOL.len() as u8];
    out.extend_from_slice(PROTOCOL);
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(info_hash);
    out.extend_from_slice(peer_id);
    stream.write_all(&out)
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        let u32s = |b: &mut Vec<u8>, xs: &[u32]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_be_bytes()));
        match self {
            Message::KeepAlive => return vec![0, 0, 0, 0],
            Message::Choke => body.push(0),
            Message::Unchoke => body.push(1),
            Message::Interested => body.push(2),
            Message::NotInterested => body.push(3),
            Message::Have(i) => {
                body.push(4);
                u32s(&mut body, &[*i]);
            }
            Message::Bitfield(b) => {
                body.push(5);
                body.extend_from_slice(b);
            }
            Message::Request { index, begin, length } => {
                body.push(6);
                u32s(&mut body, &[*index, *begin, *length]);
            }
            Message::Piece { index, begin, block } => {
                body.push(7);
                u32s(&mut body, &[*index, *begin]);
                body.extend_from_slice(block);
            }
            Message::Cancel { index, begin, length } => {
                body.push(8);
                u32s(&mut body, &[*index, *begin, *length]);
            }
            Message::Unknown(id) => body.push(*id),
        }
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        out.extend(body);
        out
    }

    pub fn decode(body: &[u8]) -> io::Result<Message> {
        if body.is_empty() {
            return Ok(Message::KeepAlive);
        }
        let u = |i: usize| -> io::Result<u32> {
            body.get(1 + i * 4..5 + i * 4).map(|s| u32::from_be_bytes(s.try_into().unwrap())).ok_or_else(|| bad("short message"))
        };
        Ok(match body[0] {
            0 => Message::Choke,
            1 => Message::Unchoke,
            2 => Message::Interested,
            3 => Message::NotInterested,
            4 => Message::Have(u(0)?),
            5 => Message::Bitfield(body[1..].to_vec()),
            6 => Message::Request { index: u(0)?, begin: u(1)?, length: u(2)? },
            7 => Message::Piece { index: u(0)?, begin: u(1)?, block: body.get(9..).ok_or_else(|| bad("short piece"))?.to_vec() },
            8 => Message::Cancel { index: u(0)?, begin: u(1)?, length: u(2)? },
            id => Message::Unknown(id),
        })
    }
}

pub fn write_msg(stream: &mut impl Write, m: &Message) -> io::Result<()> {
    stream.write_all(&m.encode())
}

pub fn read_msg(stream: &mut impl Read) -> io::Result<Message> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > 2 * BLOCK as usize + 64 && len > 1 << 20 {
        return Err(bad("message too large"));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Message::decode(&body)
}

pub fn has_piece(bitfield: &[u8], index: usize) -> bool {
    bitfield.get(index / 8).is_some_and(|b| b & (0x80 >> (index % 8)) != 0)
}

pub fn set_piece(bitfield: &mut [u8], index: usize) {
    if let Some(b) = bitfield.get_mut(index / 8) {
        *b |= 0x80 >> (index % 8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roundtrip() {
        let msgs = [
            Message::Choke,
            Message::Interested,
            Message::Have(9),
            Message::Bitfield(vec![0b1010_0000]),
            Message::Request { index: 1, begin: 16384, length: 16384 },
            Message::Piece { index: 2, begin: 0, block: vec![1, 2, 3] },
        ];
        for m in msgs {
            let enc = m.encode();
            assert_eq!(read_msg(&mut &enc[..]).unwrap(), m);
        }
        assert_eq!(read_msg(&mut &[0u8, 0, 0, 0][..]).unwrap(), Message::KeepAlive);
    }

    #[test]
    fn bitfield_ops() {
        let mut bf = vec![0u8; 2];
        set_piece(&mut bf, 0);
        set_piece(&mut bf, 9);
        assert!(has_piece(&bf, 0) && has_piece(&bf, 9) && !has_piece(&bf, 1));
        assert_eq!(bf, vec![0x80, 0x40]);
    }
}
