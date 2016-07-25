use std::collections::HashMap;
use std::sync::{Arc, RwLock, Mutex};

use engine_io::socket;
use packet::{Packet, Opcode};
use serde_json:Value;

#[derive(Clone)]
pub struct Socket {
    socket: socket::Socket,
    callbacks: Arc<RwLock<HashMap<Value, Box<Fn(Value, Option<Vec<Vec<u8>>>) + 'static>>>>,
    acks: Arc<Mutex<HashMap<usize, Box<Fn(Vec<u8>)>>>>,
    rooms_joined: Arc<RwLock<Vec<String>>>,
    server_rooms: Arc<RwLock<HashMap<String, Vec<Socket>>>>,
    cur_packet: Arc<RwLock<Option<Packet>>>,
}

unsafe impl Send for Socket{}
unsafe impl Sync for Socket{}

impl Socket {
    #[doc(hidden)]
    pub fn new(socket: socket::Socket, server_rooms:
               Arc<RwLock<HashMap<String, Vec<Socket>>>>) -> Socket {
        let so = Socket {
            socket: socket,
            callbacks: Arc::new(RwLock::new(HashMap::new())),
            rooms_joined: Arc::new(RwLock::new(Vec::new())),
            server_rooms: server_rooms,
            cur_packet: Arc::new(RwLock::new(None)),
        };
        let cl = so.clone();

        socket.on_message(|bytes| {
            if so.has_buffered_packet() {
                let mut packet = so.cur_packet.write().unwrap();
                if packet.add_attachment(bytes.into_vec()) {
                    // received all attachments, fire relevant
                    // callback/ack
                    
                } else {
                    return;
                }
            }

            let packet = try!(Packet::from_bytes(bytes).map_err(|_| ()));
	    if packet.attachments != 0 {
                if packet.opcde != Opcode::BinaryEvent ||
                    packet.opcode != Opcode::BinaryAck {
                        // since only BinaryEvent and BinaryAck
                        // can have attachments
                        return;
                    }
	        // packet has an attachment
	        let mut cur = so.cur_packet.write().unwrap();
                *cur = Some(packet);
                return;
	    }
        });

        cl
    }

    fn fire_callback(&self, packet: &Packet) {
        let callbacks = self.callbacks.read().unwrap();
        
    }
    
    fn has_buffered_packet(&self) -> bool {
        let cur = so.cur_packet.read().unwrap();
        cur.is_some()
    }

    pub fn id(&self) -> String {
        self.socket.id()
    }

    pub fn on<F>(&self, event: String, f: F)
        where F: Fn(Vec<u8>, Option<Vec<Vec<u8>>>) + 'static {
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
    pub fn send(&self, data: Vec<u8>) {
        self.socket.send(data);
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
                    break
                }
            }

            clients.swap_remove(i);
        }
    }
}
