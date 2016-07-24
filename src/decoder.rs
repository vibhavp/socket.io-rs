use packet::{Packet, Opcode};
use serde_json::Value;

pub struct Decoder {
    cur_packet: Option<Packet>,
    buffered_event: Option<Event>,
}

pub enum Data {
    Binary(Vec<u8>),
    Text(Value)
}

pub struct Event {
    event: Data,
    params: Vec<Data>,
}

fn to_array(value: Value) -> Vec<Value> {
    if let Value::Array(v) = value {
        v
    } else {
        panic!("non-array Value passed to to_array()");
    }
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder {
            cur_packet: None,
            buffered_event: None,
        }
    }

    fn decode_bytes(bytes: &[u8]) {
        
    }
    
    fn decode_packet(&mut self, packet: Packet) -> Option<Event> {
        match packet.opcode {
            Opcode::Event => {
                let arr = to_array(packet.data.unwrap());
                let event = Data::Text(arr[0].clone());
                let params = arr.into_iter().skip(1).map(|d| Data::Text(d)).collect();
                
                Some(Event{
                    event: event,
                    params: params,
                })
            },
            Opcode::BinaryEvent => {
                self.buffered_event.as_mut().unwrap().params[0] = Data::Text(Value::I64(123));
                None
            },
            _ => unreachable!()
        }
    }
}
