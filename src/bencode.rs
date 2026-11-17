//! Bencode (BEP 3) decoder and encoder.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Dict(BTreeMap<Vec<u8>, Value>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError(pub String, pub usize);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bencode: {} at byte {}", self.0, self.1)
    }
}
impl std::error::Error for DecodeError {}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(d) => d.get(key.as_bytes()),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        self.as_bytes().and_then(|b| std::str::from_utf8(b).ok())
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Value::Int(i) => out.extend_from_slice(format!("i{i}e").as_bytes()),
            Value::Bytes(b) => {
                out.extend_from_slice(format!("{}:", b.len()).as_bytes());
                out.extend_from_slice(b);
            }
            Value::List(l) => {
                out.push(b'l');
                l.iter().for_each(|v| v.encode_into(out));
                out.push(b'e');
            }
            Value::Dict(d) => {
                out.push(b'd');
                for (k, v) in d {
                    Value::Bytes(k.clone()).encode_into(out);
                    v.encode_into(out);
                }
                out.push(b'e');
            }
        }
    }
}

/// Convenience constructors used when building torrents.
pub fn dict<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Dict(pairs.into_iter().map(|(k, v)| (k.as_bytes().to_vec(), v)).collect())
}
pub fn bytes(b: impl Into<Vec<u8>>) -> Value {
    Value::Bytes(b.into())
}

pub fn decode(input: &[u8]) -> Result<Value, DecodeError> {
    let mut d = Decoder { input, pos: 0, depth: 0 };
    let v = d.value()?;
    if d.pos != input.len() {
        return Err(DecodeError("trailing data".into(), d.pos));
    }
    Ok(v)
}

/// Decodes one value from the front of `input`, returning it and the number of bytes consumed.
pub fn decode_prefix(input: &[u8]) -> Result<(Value, usize), DecodeError> {
    let mut d = Decoder { input, pos: 0, depth: 0 };
    let v = d.value()?;
    Ok((v, d.pos))
}

struct Decoder<'a> {
    input: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Decoder<'_> {
    fn err<T>(&self, msg: &str) -> Result<T, DecodeError> {
        Err(DecodeError(msg.into(), self.pos))
    }
    fn peek(&self) -> Result<u8, DecodeError> {
        self.input.get(self.pos).copied().ok_or_else(|| DecodeError("unexpected end".into(), self.pos))
    }
    fn until(&mut self, end: u8) -> Result<&[u8], DecodeError> {
        let start = self.pos;
        let rel = self.input[start..].iter().position(|&b| b == end).ok_or_else(|| DecodeError("unterminated".into(), start))?;
        self.pos = start + rel + 1;
        Ok(&self.input[start..start + rel])
    }
    fn int_from(&self, raw: &[u8]) -> Result<i64, DecodeError> {
        let s = std::str::from_utf8(raw).map_err(|_| DecodeError("non-ascii int".into(), self.pos))?;
        if s.is_empty() || (s.len() > 1 && (s.starts_with('0') || s.starts_with("-0"))) || s == "-" {
            return self.err("invalid integer");
        }
        s.parse().map_err(|_| DecodeError("invalid integer".into(), self.pos))
    }

    fn value(&mut self) -> Result<Value, DecodeError> {
        self.depth += 1;
        if self.depth > 256 {
            return self.err("nesting too deep");
        }
        let v = match self.peek()? {
            b'i' => {
                self.pos += 1;
                let raw = self.until(b'e')?.to_vec();
                Value::Int(self.int_from(&raw)?)
            }
            b'l' => {
                self.pos += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    items.push(self.value()?);
                }
                self.pos += 1;
                Value::List(items)
            }
            b'd' => {
                self.pos += 1;
                let mut map = BTreeMap::new();
                while self.peek()? != b'e' {
                    let Value::Bytes(k) = self.value()? else { return self.err("dict key must be a string") };
                    let v = self.value()?;
                    map.insert(k, v);
                }
                self.pos += 1;
                Value::Dict(map)
            }
            b'0'..=b'9' => {
                let raw = self.until(b':')?.to_vec();
                let len = self.int_from(&raw)? as usize;
                let end = self.pos.checked_add(len).filter(|&e| e <= self.input.len());
                let Some(end) = end else { return self.err("string longer than input") };
                let b = self.input[self.pos..end].to_vec();
                self.pos = end;
                Value::Bytes(b)
            }
            _ => return self.err("unexpected byte"),
        };
        self.depth -= 1;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_basics() {
        assert_eq!(decode(b"i42e").unwrap(), Value::Int(42));
        assert_eq!(decode(b"i-3e").unwrap(), Value::Int(-3));
        assert_eq!(decode(b"4:spam").unwrap(), bytes("spam"));
        assert_eq!(decode(b"l4:spami7ee").unwrap(), Value::List(vec![bytes("spam"), Value::Int(7)]));
        let d = decode(b"d3:cow3:moo4:spam4:eggse").unwrap();
        assert_eq!(d.get("cow").unwrap().as_str(), Some("moo"));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [&b"i03e"[..], b"i-0e", b"ie", b"5:abc", b"l", b"d1:ae", b"x", b"i1ei2e"] {
            assert!(decode(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn roundtrip_is_canonical() {
        let raw = b"d8:announce3:url4:infod6:lengthi10e4:name3:abcee";
        assert_eq!(decode(raw).unwrap().encode(), raw.to_vec());
    }
}
