use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, RwLock, Mutex};

use engine_io::socket;
use serde_json::Value;
use data::{encode_data, Data};
use packet::{Packet, Opcode};

#[derive(Clone)]
pub struct Socket {
    socket: socket::Socket,
    callbacks: Arc<RwLock<HashMap<String, Box<Fn(Vec<Value>, Option<Vec<Vec<u8>>>) -> Vec<Data>>>>>,
    acks: Arc<Mutex<HashMap<usize, Box<Fn(Option<Value>, Option<Vec<Vec<u8>>>)>>>>,
    rooms_joined: Arc<RwLock<Vec<String>>>,
    server_rooms: Arc<RwLock<HashMap<String, Vec<Socket>>>>,
    cur_packet: Arc<RwLock<Option<Packet>>>,
    last_ack_id: Arc<AtomicUsize>,
    namespace: Option<String>,
}

unsafe impl Send for Socket {}
unsafe impl Sync for Socket {}

impl Socket {
    #[doc(hidden)]
    pub fn new(socket: socket::Socket,
               server_rooms: Arc<RwLock<HashMap<String, Vec<Socket>>>>)
               -> Socket {
        let so = Socket {
            socket: socket.clone(),
            callbacks: Arc::new(RwLock::new(HashMap::new())),
            acks: Arc::new(Mutex::new(HashMap::new())),
            rooms_joined: Arc::new(RwLock::new(Vec::new())),
            server_rooms: server_rooms,
            namespace: None,
            cur_packet: Arc::new(RwLock::new(None)),
            last_ack_id: Arc::new(AtomicUsize::new(0)),
        };
        let cl = so.clone();

        socket.on_message(move |bytes| {
            if so.has_buffered_packet() {
                let mut packet = so.cur_packet.write().unwrap();
                if packet.as_mut().unwrap().add_attachment(bytes.to_vec()) {
                    // received all attachments, fire relevant
                    // callback/ack
                    match packet.as_ref().unwrap().opcode {
                        Opcode::BinaryEvent => {
                            let packet = packet.take().unwrap();
                            let ack = so.fire_callback(&packet);

                            if packet.id.is_some() {
                                if ack.is_some() {
                                    let (json, binary) = encode_data(ack.unwrap());
                                    so.send(json.to_string().into_bytes());
                                    for b in binary {
                                        so.send(b);
                                    }
                                } else {
                                    so.send("[]".to_string().into_bytes());
                                }
                            }
                        }
                        Opcode::BinaryAck => {
                            // fire ack callback
                            so.fire_ack(packet.take().unwrap());
                        }
                        _ => unreachable!(),
                    }
                } else {
                    return;
                }
            }

            let packet = match Packet::from_bytes(bytes) {
                Ok(p) => p,
                Err(_) => return, //TODO: emit error here
            };
            if packet.has_attachments() {
                if packet.opcode == Opcode::BinaryEvent || packet.opcode == Opcode::BinaryAck {
                    // BinaryEvent and BinaryAck
                    // can have attachments
                    let mut cur = so.cur_packet.write().unwrap();
                    *cur = Some(packet);
                }
                return;
            }
        });

        cl
    }

    fn fire_callback(&self, packet: &Packet) -> Option<Vec<Data>> {
        let event_arr: &Vec<Value> = match packet.data.as_ref().unwrap() {
            &Value::Array(ref v) => v,
            _ => panic!("Event packet doesn't have an array payload"),
        };

        let ref event = event_arr[0];

        let callbacks = self.callbacks.read().unwrap();
        if let Some(func) = callbacks.get(&event.to_string()) {
            Some(func(event_arr.into_iter().skip(1).map(|v| v.clone()).collect(),
                      packet.get_attachments()))
        } else {
            None
        }
    }

    fn fire_ack(&self, packet: Packet) {
        let map = self.acks.lock();
        if let Some(callback) = map.unwrap().remove(&packet.id.unwrap()) {
            callback(packet.data.clone(), packet.get_attachments().clone());
        }
    }

    fn has_buffered_packet(&self) -> bool {
        let cur = self.cur_packet.read().unwrap();
        cur.is_some()
    }

    pub fn id(&self) -> String {
        self.socket.id()
    }

    pub fn on<F>(&self, event: String, f: F)
        where F: Fn(Vec<Value>, Option<Vec<Vec<u8>>>) -> Vec<Data> + 'static
    {
        let mut map = self.callbacks.write().unwrap();
        map.insert(event, Box::new(f));
    }

    pub fn join(&self, room: String) {
        let mut rooms = self.rooms_joined.write().unwrap();
        if !rooms.contains(&room) {
            rooms.push(room.clone());
            drop(rooms);

            let mut map = self.server_rooms.write().unwrap();
            if map.contains_key(&room) {
                map.get_mut(&room).unwrap().push(self.clone())
            } else {
                map.insert(room, vec![self.clone()]);
            }
        }
    }

    #[inline(always)]
    #[doc(hidden)]
    pub fn send(&self, data: Vec<u8>) {
        self.socket.send(data);
    }

    pub fn emit(&self, event: Value, params: Option<Vec<Data>>) {
        let mut all_event_params: Vec<_> = vec![Data::JSON(event)];
        if params.is_some() {
            all_event_params.extend_from_slice(&params.unwrap());
        }

        let (json, binary_vec) = encode_data(all_event_params);
        self.send(Packet::new_event(self.namespace.clone(), None, binary_vec.len(), json)
            .encode()
            .into_bytes());
        for binary in binary_vec {
            self.send(binary);
        }
    }

    pub fn emit_ack<F>(&self, event: Value, params: Option<Vec<Data>>, on_ack: F)
        where F: Fn(Option<Value>, Option<Vec<Vec<u8>>>) + 'static
    {
        let mut all_event_params: Vec<_> = vec![Data::JSON(event)];
        if params.is_some() {
            all_event_params.extend_from_slice(&params.unwrap());
        }

        let ack_id = self.new_ack_id();
        {
            let mut map = self.acks.lock().unwrap();
            map.insert(ack_id, Box::new(on_ack));
        }
        let (json, binary_vec) = encode_data(all_event_params);
        self.send(Packet::new_event(self.namespace.clone(), Some(ack_id), binary_vec.len(), json)
            .encode()
            .into_bytes());
        for binary in binary_vec {
            self.send(binary);
        }
    }

    fn new_ack_id(&self) -> usize {
        self.last_ack_id.fetch_add(1, Relaxed)
    }

    pub fn close(&mut self) {
        self.socket.close("close()");
        let rooms_joined = self.rooms_joined.read().unwrap();

        for room in rooms_joined.iter() {
            let mut map = self.server_rooms.write().unwrap();
            let mut clients = map.get_mut(room).unwrap();
            let mut i = 0;

            for (index, so) in clients.iter().enumerate() {
                if so.id() == self.id() {
                    i = index;
                    break;
                }
            }

            clients.swap_remove(i);
        }
    }
}
