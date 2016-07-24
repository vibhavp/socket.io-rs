use std::mem;
use std::string::FromUtf8Error;
use std::convert::From;
use std::iter::Peekable;

use engine_io::packet;
use serde_json::ser::to_string;
use serde_json::de::from_str;
use serde_json::error::Error as JSONError;
use serde_json::Value;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Opcode {
    Connect = '0' as isize,
    Disconnect = '1' as isize,
    Event = '2' as isize,
    Ack = '3' as isize,
    Error = '4' as isize,
    BinaryEvent = '5' as isize,
    BinaryAck = '6' as isize
}

#[derive(Clone, PartialEq, Debug)]
pub struct Packet {
    pub namespace: Option<String>,
    pub opcode: Opcode,
    pub id: Option<usize>,
    /// Number of attachments
    pub attachments: usize,
    pub data: Option<Value>,
}

#[derive(Debug)]
pub enum Error {
    InvalidOpcode(u8),
    InvalidPacket,
    PacketDataNotArray,
    JSONError(JSONError),
    FromUtf8Error(FromUtf8Error),
}

impl From<JSONError> for Error {
    fn from(e: JSONError) -> Error {
        Error::JSONError(e)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(e: FromUtf8Error) -> Error {
        Error::FromUtf8Error(e)
    }
}

impl Packet {
    pub fn from_bytes(bytes: &[u8]) -> Result<Packet, Error> {
        let mut chars: Peekable<_> = bytes.iter().peekable();

        let opcode: Opcode = match chars.next() {
            Some(c) if *c > (Opcode::BinaryAck as u8) =>
                return Err(Error::InvalidOpcode(*c as u8)),
            Some(c) => unsafe{mem::transmute(*c as u8)},
            None => return Err(Error::InvalidPacket),
        };

        let mut attachments = 0;
        if opcode == Opcode::BinaryAck || opcode == Opcode::BinaryEvent {
            while let Some(c) = chars.next() {
                if chars.len() == 0 {
                    return Err(Error::InvalidPacket)
                }
                if *c == '-' as u8 {
                    break;
                }
                attachments = 10 * attachments +
                    try!((*c as char).to_digit(10)
                         .ok_or(Error::InvalidPacket)) as usize;
            }
        }

        let nsp = if chars.peek().map_or(false, |ch| **ch == '/' as u8) {
            let s = try!(String::from_utf8(chars.by_ref().
                                           take_while(|c| **c != b',')
                                           .map(|c| *c).collect()));
            Some(s)
        } else {
            None
        };

        let mut id: usize = 0;
        let mut has_id = false;

        loop {
            if chars.peek().map_or(false, |ch: &&u8| **ch >= b'0' && **ch <= b'9') {
                id = id * 10 + (*chars.next().unwrap() as char).to_digit(10).unwrap() as usize;
                has_id = true;
            } else {
                break;
            }
        }

        let data: Option<Value> = match opcode {
            Opcode::Event | Opcode::BinaryEvent | Opcode::Ack | Opcode::BinaryAck
                => {
                    let js = try!(String::from_utf8(chars.map(|c| *c).collect()));
                    let parsed: Value = try!(from_str(&js));

                    if (opcode == Opcode::Event || opcode == Opcode::BinaryEvent)
                        && !parsed.is_array() {
                        return Err(Error::PacketDataNotArray);
                    }

                    Some(parsed)
                },
            _ => None,
        };

        Ok(Packet {
            namespace: nsp,
            attachments: attachments,
            opcode: opcode,
            id: if has_id {Some(id)} else {None},
            data: data
        })
    }
}

impl Packet {
    pub fn encode(&self) -> String {
        let mut s = String::new();
        let mut nsp = false;

        s.push((self.opcode as u8) as char);
        if self.attachments != 0 {
            s.push_str(&self.attachments.to_string());
            s.push('-');
        }
        if let Some(ref n) = self.namespace {
            s.push_str(n);
            nsp = true;
        }

        if let Some(id) = self.id {
            if nsp {
                s.push(',');
                nsp = false;
            }
            s.push_str(&id.to_string());
        }

        if let Some(ref data) = self.data {
            if nsp {
                s.push(',');
            }
            s.push_str(&to_string(data).unwrap());
        }

        s
    }

    /// Encode the packet to a engine.io `Packet`.
    pub fn encode_to_engine_packet(&self) -> packet::Packet {
        packet::Packet{
            id: packet::ID::Message,
            data: self.encode().into_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Opcode::*;
    use super::Packet;
    use serde_json::value::to_value;

    macro_rules! packet {
        ((data $data:expr ); $(( $x:ident $y:expr ));*) => {
            {
                let mut packet = Packet{
                    namespace: None,
                    attachments: 0,
                    opcode: Event,
                    id: None,
                    data: Some(to_value($data)),
                };
                $(
                    packet.$x = $y;
                )*;
                packet
            }
        };

        ($(( $x:ident $y:expr ));+) => {
            {
                let mut packet = packet!();
                $(
                    packet.$x = $y;
                )*;
                packet
            }
        };

        () => {
            {
                Packet {
                    namespace: None,
                    attachments: 0,
                    opcode: Connect,
                    id: None,
                    data: None,
                }
            }
        }
    }

    macro_rules! test {
        ($name: ident, $packet: expr, $output: expr) => {
            #[test]
            fn $name() {
                test_encoding(&$packet, $output);
                test_decoding($output, &$packet);
            }
        };
    }

    fn test_encoding(packet: &Packet, output: &str) {
        let encoding = packet.encode();
        assert_eq!(encoding, output);
    }

    fn test_decoding(s: &str, packet: &Packet) {
        let decoded = Packet::from_bytes(s.as_bytes()).expect("Decoding packet");
        assert_eq!(&decoded, packet);
    }

    test!(type_data, packet!((opcode Connect)), "0");
    test!(type_namespace_data, packet!((namespace Some("/abc".to_string()));
                                       (opcode Connect))
          , "0/abc");
    test!(namespace_vec, packet!((data &vec!["foo", "bar"]);
                                 (namespace Some("/foo".to_string()))),
          "2/foo,[\"foo\",\"bar\"]");
    test!(id_namespace_vec, packet!((data &vec![1,2,3,4]);
                                    (namespace Some("/abc".to_string()));
                                    (id Some(1))), "2/abc,1[1,2,3,4]");
    test!(attachment, packet!((data &vec![1]);
                              (attachments 1);
                              (opcode BinaryEvent)), "51-[1]");
}
