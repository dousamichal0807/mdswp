use std::collections::HashMap;
use std::collections::VecDeque;
use std::io;
use std::convert::TryInto;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use rand::Rng;

use crate::MdswpStream;
use crate::segment::MAX_SEGMENT_LEN;
use crate::segment::Segment;
use crate::stream::inner::StreamInner;
use crate::util::{clone, clone_io_err};
use crate::util::sock_addr_v4_any;
use crate::util::sock_addr_v6_any;

pub struct ListenerInner {
    socket: UdpSocket,
    thread: RwLock<Option<JoinHandle<()>>>,
    connections: RwLock<HashMap<SocketAddr, Weak<StreamInner>>>,
    to_accept: RwLock<VecDeque<Arc<StreamInner>>>,
    error: RwLock<io::Result<()>>,
}

impl ListenerInner {
    pub fn bind<A>(addr: A) -> io::Result<Arc<Self>>
        where A: ToSocketAddrs {
        let socket = UdpSocket::bind(addr)?;
        Self::__new(socket, true)
    }

    pub(crate) fn _new_for_single(peer_addr: SocketAddr) -> io::Result<Arc<Self>> {
        let local_addr = match peer_addr {
            SocketAddr::V4(..) => sock_addr_v4_any(),
            SocketAddr::V6(..) => sock_addr_v6_any()
        };
        let socket = UdpSocket::bind(local_addr)?;
        socket.connect(peer_addr)?;
        Self::__new(socket, false)
    }

    fn __new(socket: UdpSocket, start_thread: bool) -> io::Result<Arc<Self>> {
        let instance = Arc::new(Self {
            socket,
            thread: RwLock::new(Option::None),
            connections: RwLock::new(HashMap::new()),
            to_accept: RwLock::new(VecDeque::new()),
            error: RwLock::new(Result::Ok(())),
        });
        if start_thread {
            instance._start_thread()?;
        }
        Result::Ok(instance)
    }

    pub fn is_err(&self) -> bool {
        self.error.read().unwrap().is_err()
    }

    pub(crate) fn listener_state(&self) -> io::Result<()> {
        match &*self.error.read().unwrap() {
            Result::Ok(()) => Result::Ok(()),
            Result::Err(err) => Result::Err(clone_io_err(&err))
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn accept_nonblocking(&self) -> io::Result<Option<(MdswpStream, SocketAddr)>> {
        match &*self.error.read().unwrap() {
            Result::Ok(()) => match self.to_accept.write().unwrap().pop_front() {
                Option::Some(conn) => {
                    let addr = conn.peer_addr()?;
                    let conn = MdswpStream(conn);
                    Result::Ok(Option::Some((conn, addr)))
                },
                Option::None => Result::Ok(Option::None),
            },
            Result::Err(err) => Result::Err(io::Error::new(err.kind(), err.to_string()))
        }
    }

    pub fn accept(&self) -> io::Result<(MdswpStream, SocketAddr)> {
        loop {
            match self.accept_nonblocking() {
                Result::Ok(Option::None) => thread::sleep(Duration::ZERO),
                Result::Ok(Option::Some(conn_tuple)) => return Result::Ok(conn_tuple),
                Result::Err(err) => return Result::Err(err)
            }
        }
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.socket.set_ttl(ttl)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        self.socket.ttl()
    }

    pub(crate) fn _start_thread(self: &Arc<Self>) -> io::Result<()> {
        self.socket.set_nonblocking(false)?;
        let weak = Arc::downgrade(&self);
        let thread = thread::spawn(clone!(weak => || Self::__listen_thread(weak)));
        *self.thread.write().unwrap() = Option::Some(thread);
        Result::Ok(())
    }

    pub(crate) fn _add_connection(self: &Arc<Self>, peer_addr: SocketAddr, stream: Weak<StreamInner>) {
        self.connections.write().unwrap().insert(peer_addr, stream);
    }

    pub(crate) fn _remove_connection(self: &Arc<Self>, peer_addr: SocketAddr) {
        self.connections.write().unwrap().remove(&peer_addr);
    }

    pub(crate) fn _connect_listener_to(&self, addr: SocketAddr) -> io::Result<()> {
        self.socket.connect(addr)
    }

    pub(crate) fn _send_to(&self, addr: SocketAddr, segment: Segment) -> io::Result<()> {
        self.socket.send_to(&segment.into_bytes(), addr).and(Result::Ok(()))
    }

    pub(crate) fn _try_recv(&self) -> io::Result<Option<Segment>> {
        let mut buf = [0; MAX_SEGMENT_LEN + 1];
        let len = match self.socket.recv(&mut buf[..]) {
            Result::Ok(0) => return Result::Ok(Option::None),
            Result::Ok(n) => n,
            Result::Err(err) => return match err.kind() {
                io::ErrorKind::Interrupted => Result::Ok(Option::None),
                io::ErrorKind::WouldBlock => Result::Ok(Option::None),
                _other => Result::Err(err)
            }
        };
        match (&buf[..len]).try_into() {
            Result::Ok(segment) => Result::Ok(Option::Some(segment)),
            Result::Err(err) => Result::Err(err)
        }
    }

    fn __error(&self, err: io::Error) {
        *self.error.write().unwrap() = Result::Err(err);
    }

    fn __listen_thread(weak: Weak<Self>) {
        let mut buf = [0; MAX_SEGMENT_LEN + 1];
        loop {
            // Try upgrade from weak to strong reference. If it fails, the listener
            // was dropped, so return immediately:
            let this = match weak.upgrade() {
                Option::Some(arc) => arc,
                Option::None => return
            };
            // If the listener errored, return immediately:
            if this.is_err() { return; }
            // Try to receive a datagram:
            match this.socket.recv_from(&mut buf) {
                Result::Err(err) => match err.kind() {
                    io::ErrorKind::Interrupted => {},
                    io::ErrorKind::WouldBlock => unreachable!(),
                    _other => this.__error(err),
                }
                Result::Ok((n, addr)) => this.__recv(&buf[..n], addr)
            }
        }
    }

    fn __recv(self: &Arc<Self>, data: &[u8], addr: SocketAddr) {
        // Try to get a connection
        let conn = match self.connections.read().unwrap().get(&addr) {
            Option::Some(weak) => weak.upgrade(),
            Option::None => Option::None
        };
        // If connection is not present yet, create a new one:
        match conn {
            Option::Some(conn) => match data.try_into() {
                Result::Err(err) => conn.error(err, true),
                Result::Ok(segment) => conn.recv_segment(segment),
            },
            Option::None => match data.try_into() {
                Result::Ok(Segment::Establish { start_seq_num: establish_seq_num }) => {
                    let accept_seq_num = rand::thread_rng().gen();
                    let conn = StreamInner::new_by_peer(self.clone(), addr, establish_seq_num, accept_seq_num);
                    self.connections.write().unwrap().insert(addr, Arc::downgrade(&conn));
                    self.to_accept.write().unwrap().push_back(conn.clone());
                },
                _other => { let _ = self._send_to(addr, Segment::Reset); },
            }
        };
    }
}