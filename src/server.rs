use std::sync::{Arc, RwLock};
use std::collections::HashMap;

use socket::Socket;
use engine_io::server;
use iron::prelude::*;
use iron::middleware::Handler;

#[derive(Clone)]
pub struct Server {
    server: server::Server,
    clients: Arc<RwLock<Vec<Socket>>>,
    server_rooms: Arc<RwLock<HashMap<String, Vec<Socket>>>>,
    on_connection: Arc<RwLock<Option<Box<Fn(Socket) + 'static>>>>,
}

unsafe impl Send for Server {}
unsafe impl Sync for Server {}

impl Server {
    /// Returns a socket.io `Server` instance from an engine.io `Server` instance.
    pub fn from_server(server: server::Server) -> Server {
        let socketio_server = Server {
            server: server.clone(),
            clients: Arc::new(RwLock::new(vec![])),
            server_rooms: Arc::new(RwLock::new(HashMap::new())),
            on_connection: Arc::new(RwLock::new(None)),
        };

        let cl1 = socketio_server.clone();

        server.on_connection(move |so| {
            let socketio_socket = Socket::new(so.clone(), socketio_server.server_rooms.clone());

            {
                let mut rooms = socketio_server.server_rooms.write().unwrap();
                rooms.insert(so.id(), vec![socketio_socket.clone()]);
            }
            {
                let mut clients = socketio_server.clients.write().unwrap();
                clients.push(socketio_socket.clone());
            }
            socketio_server.on_connection
                .read()
                .unwrap()
                .as_ref()
                .map(|func| func(socketio_socket));
        });

        cl1
    }

    #[inline(always)]
    pub fn new() -> Server {
        Server::from_server(server::Server::new())
    }

    #[inline(always)]
    pub fn on_connection<F>(&self, f: F)
        where F: Fn(Socket) + 'static
    {
        *self.on_connection.write().unwrap() = Some(Box::new(f));
    }

    pub fn close(&mut self) {
        let mut clients = self.clients.write().unwrap();
        for so in clients.iter_mut() {
            so.close();
        }
    }
}


impl Handler for Server {
    #[inline(always)]
    fn handle(&self, req: &mut Request) -> IronResult<Response> {
        self.server.handle(req)
    }
}
