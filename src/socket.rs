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
    namespace: Arc<RwLock<Option<String>>>,
    on_close: Arc<RwLock<Option<Box<Fn()>>>>,
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
            namespace: Arc::new(RwLock::new(None)),
            cur_packet: Arc::new(RwLock::new(None)),
            last_ack_id: Arc::new(AtomicUsize::new(0)),
            on_close: Arc::new(RwLock::new(None)),
        };
        let cl = so.clone();

        socket.on_message(move |bytes| {
            if so.has_buffered_packet() {
                let mut packet = so.cur_packet.write().unwrap();
                if packet.as_mut().unwrap().add_attachment(bytes.to_vec()) {
                    // received all attachments, fire relevant
                    // callback/ack
                    let packet = packet.take().unwrap();
                    match packet.opcode {
                        Opcode::BinaryEvent => {
                            let ack = so.fire_callback(&packet);

                            if let Some(id) = packet.id {
                                if let Some(ack) = ack {
                                    let (json, binary) = encode_data(ack);
                                    so.send_ack(id, json, binary);
                                } else {
                                    so.send("[]".to_string().into_bytes());
                                }
                            }
                        }
                        Opcode::BinaryAck => so.fire_ack(&packet),
                        _ => unreachable!(),
                    }
                } else {
                    return;
                }
            }

            let packet: Packet = match Packet::from_bytes(bytes) {
                Ok(p) => p,
                Err(e) => {
                    so.send(Packet::new_error(so.namespace.read().unwrap().clone(),
                                              e).encode().into_bytes());
                    return;
                }, //TODO: emit error here
            };

            match packet.opcode {
                Opcode::Disconnect => {so.clone().close(); return;},
                Opcode::Event => {
                    let ack = so.fire_callback(&packet);

                    if let Some(id) = packet.id {
                        if let Some(ack) = ack {
                            let (json, binary) = encode_data(ack);
                            so.send_ack(id, json, binary);
                        } else {
                            so.send("[]".to_string().into_bytes());
                        }
                    }
                }
                Opcode::Ack => so.fire_ack(&packet),
                Opcode::Connect => {
                    *so.namespace.write().unwrap() = packet.namespace.clone();
                },
                _ => {},
            }

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

        let so2 = cl.clone();
        socket.on_close(move |_| {
            if let Some(ref func) = *so2.on_close.read().unwrap() {
                func();
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

    fn fire_ack(&self, packet: &Packet) {
        let map = self.acks.lock();
        if let Some(callback) = map.unwrap().remove(&packet.id.unwrap()) {
            callback(packet.data.clone(), packet.get_attachments().clone());
        }
    }

    #[inline]
    fn has_buffered_packet(&self) -> bool {
        let cur = self.cur_packet.read().unwrap();
        cur.is_some()
    }

    #[inline(always)]
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

            let mut map = self.server_rooms.write().unwrap();
            if map.contains_key(&room) {
                map.get_mut(&room).unwrap().push(self.clone())
            } else {
                map.insert(room, vec![self.clone()]);
            }
        }
    }

    pub fn leave(&self, room: String) {
        let mut rooms_map = self.server_rooms.write().unwrap();
        if let Some (_) = rooms_map.remove(&room) {
            let mut rooms = self.rooms_joined.write().unwrap();
        }
    }

    fn send_ack(&self, id: usize, json: Value, attachments: Vec<Vec<u8>>) {
        self.send(Packet::new_ack(self.namespace.read().unwrap().clone(), id, attachments.len(), json).encode()
                  .into_bytes());
        for b in attachments {
            self.send(b);
        }
    }

    #[inline(always)]
    #[doc(hidden)]
    pub fn send(&self, data: Vec<u8>) {
        self.socket.send(data);
    }

    /// Emit an event to the client, with the name `event`.
    pub fn emit(&self, event: Value, params: Option<Vec<Data>>) {
        let mut all_event_params: Vec<_> = vec![Data::JSON(event)];
        if params.is_some() {
            all_event_params.extend_from_slice(&params.unwrap());
        }

        let (json, binary_vec) = encode_data(all_event_params);
        self.send(Packet::new_event(self.namespace.read().unwrap().clone(), None, binary_vec.len(), json)
            .encode()
            .into_bytes());
        for binary in binary_vec {
            self.send(binary);
        }
    }

    /// Emit an event to the client, and ask the client for an
    /// acknowledgment. Once received, call `on_ack`.
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
        self.send(Packet::new_event(self.namespace.read().unwrap().clone(), Some(ack_id), binary_vec.len(), json)
            .encode()
            .into_bytes());
        for binary in binary_vec {
            self.send(binary);
        }
    }

    fn new_ack_id(&self) -> usize {
        self.last_ack_id.fetch_add(1, Relaxed)
    }

    /// Close the connection to the client.
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
