use std::mem;
use std::string::FromUtf8Error;
use std::convert::From;
use std::iter::Peekable;

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
    BinaryAck = '6' as isize,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Packet {
    pub namespace: Option<String>,
    pub opcode: Opcode,
    pub id: Option<usize>,
    /// Number of attachments
    pub attachments_num: usize,
    pub data: Option<Value>,
    attachments: Option<Vec<Vec<u8>>>,
}

#[derive(Debug)]
pub enum Error {
    InvalidOpcode(u8),
    InvalidPacket,
    PacketDataNotArray,
    JSONError(JSONError),
    FromUtf8Error(FromUtf8Error),
    NoEvent,
    AckIDMissing,
    NonBinaryHasAttachments,
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
    pub fn new_event(namespace: Option<String>,
                     id: Option<usize>,
                     attachments_num: usize,
                     params: Value)
                     -> Packet {
        debug_assert!(params.is_array());
        Packet {
            namespace: namespace,
            attachments_num: attachments_num,
            opcode: if attachments_num == 0 {
                Opcode::Event
            } else {
                Opcode::BinaryEvent
            },
            id: id,
            data: Some(params),
            attachments: None,
        }
    }

    pub fn new_error(namespace: Option<String>,
                     error: Error) -> Packet {
        Packet {
            namespace: namespace,
            attachments_num: 0,
            opcode: Opcode::Error,
            id: None,
            data: Some(Value::String(format!("{:?}", error))),
            attachments: None,
        }
    }
    
    pub fn new_ack(namespace: Option<String>,
                   id: usize,
                   attachments_num: usize,
                   params: Value)
                   -> Packet {
        Packet {
            namespace: namespace,
            attachments_num: attachments_num,
            opcode: if attachments_num == 0 {
                Opcode::Ack
            } else {
                Opcode::BinaryAck
            },
            id: Some(id),
            data: Some(params),
            attachments: None
        }
    }

    #[doc(hidden)]
    pub fn add_attachment(&mut self, bytes: Vec<u8>) -> bool {
        if self.attachments.is_none() {
            self.attachments = Some(vec![]);
        }

        debug_assert!(self.attachments.as_ref().unwrap().len() != self.attachments_num);
        self.attachments.as_mut().unwrap().push(bytes);
        self.attachments.as_ref().unwrap().len() == self.attachments_num
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn get_attachments(&self) -> Option<Vec<Vec<u8>>> {
        self.attachments.clone()
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn has_attachments(&self) -> bool {
        self.attachments.is_some() || self.attachments_num != 0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Packet, Error> {
        let mut chars: Peekable<_> = bytes.iter().peekable();

        let opcode: Opcode = match chars.next() {
            Some(c) if *c > (Opcode::BinaryAck as u8) => return Err(Error::InvalidOpcode(*c as u8)),
            Some(c) => unsafe { mem::transmute(*c as u8) },
            None => return Err(Error::InvalidPacket),
        };

        let mut attachments_num = 0;
        if opcode == Opcode::BinaryAck || opcode == Opcode::BinaryEvent {
            while let Some(c) = chars.next() {
                if chars.len() == 0 {
                    return Err(Error::InvalidPacket);
                }
                if *c == '-' as u8 {
                    break;
                }
                attachments_num = 10 * attachments_num +
                                  try!((*c as char)
                    .to_digit(10)
                    .ok_or(Error::InvalidPacket)) as usize;
            }
        }

        let nsp = if chars.peek().map_or(false, |ch| **ch == '/' as u8) {
            let s = try!(String::from_utf8(chars.by_ref()
                .take_while(|c| **c != b',')
                .map(|c| *c)
                .collect()));
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
            Opcode::Event | Opcode::BinaryEvent | Opcode::Ack | Opcode::BinaryAck => {
                let js = try!(String::from_utf8(chars.map(|c| *c).collect()));
                let parsed: Value = try!(from_str(&js));

                if (opcode == Opcode::Event || opcode == Opcode::BinaryEvent) &&
                   !parsed.is_array() {
                    return Err(Error::PacketDataNotArray);
                }
                if (opcode == Opcode::Ack || opcode == Opcode::BinaryAck) && !has_id {
                    return Err(Error::AckIDMissing);
                }
                if (opcode != Opcode::BinaryAck && opcode != Opcode::BinaryEvent)
                    && attachments_num != 0 {
                        return Err(Error::NonBinaryHasAttachments);
                    }
                if parsed.as_array().unwrap().is_empty() {
                    return Err(Error::NoEvent);
                }

                Some(parsed)
            }
            _ => None,
        };

        Ok(Packet {
            namespace: nsp,
            attachments: None,
            attachments_num: attachments_num,
            opcode: opcode,
            id: if has_id {
                Some(id)
            } else {
                None
            },
            data: data,
        })
    }

    pub fn encode(&self) -> String {
        let mut s = String::new();
        let mut nsp = false;

        s.push((self.opcode as u8) as char);
        if self.attachments_num != 0 {
            s.push_str(&self.attachments_num.to_string());
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
                    attachments: None,
                    attachments_num: 0,
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
                    attachments: None,
                    attachments_num: 0,
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
                              (attachments_num 1);
                              (opcode BinaryEvent)), "51-[1]");
}
