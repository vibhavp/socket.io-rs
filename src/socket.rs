use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use engine_io::socket;
use packet::Packet;

#[derive(Clone)]
pub struct Socket {
    socket: socket::Socket,
    callbacks: Arc<RwLock<HashMap<String, Box<Fn(Vec<u8>) + 'static>>>>,
    rooms_joined: Arc<RwLock<Vec<String>>>,
    server_rooms: Arc<RwLock<HashMap<String, Vec<Socket>>>>,
}

unsafe impl Send for Socket{}
unsafe impl Sync for Socket{}

impl Socket {
    #[doc(hidden)]
    pub fn new(socket: socket::Socket, server_rooms: Arc<RwLock<HashMap<String, Vec<Socket>>>>) -> Socket {
        socket.on_message(|bytes| {
            let packet = Packet::from_bytes(bytes);
        });
        Socket {
            socket: socket,
            callbacks: Arc::new(RwLock::new(HashMap::new())),
            rooms_joined: Arc::new(RwLock::new(Vec::new())),
            server_rooms: server_rooms,
        }
    }

    pub fn id(&self) -> String {
        self.socket.id()
    }

    pub fn on<F>(&self, event: String, f: F)
        where F: Fn(Vec<u8>) + 'static {
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
