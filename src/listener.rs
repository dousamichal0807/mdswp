//! Module for [`MdswpListener`](crate::MdswpListener) and its related data
//! structures.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::convert::TryInto;
use std::io;
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
use crate::util::clone;
use crate::util::sock_addr_v4_any;
use crate::util::sock_addr_v6_any;

/// A MDSWP socket, listening for connections.
///
/// After creating a [`MdswpStream`] by binding it to a socket address, it listens
/// for incoming TCP connections. These can be accepted by calling
/// [`accept`](Self::accept) or by iterating over the [`Incoming`] iterator returned
/// by [`incoming`](Self::incoming) method.
///
/// The socket will be closed when the value is dropped.
pub struct MdswpListener {
    socket: UdpSocket,
    thread: RwLock<Option<JoinHandle<()>>>,
    connections: RwLock<HashMap<SocketAddr, Weak<StreamInner>>>,
    to_accept: RwLock<VecDeque<Arc<StreamInner>>>,
    error: RwLock<io::Result<()>>,
}

impl MdswpListener {
    /// Creates a new [`MdswpListener`] which will be bound to the specified
    /// address.
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port to
    /// this listener. The port allocated can be queried via the
    /// [`local_addr`](Self::local_addr) method.
    ///
    /// The address type can be any implementor of [`ToSocketAddrs`] trait. See its
    /// documentation for concrete examples.
    ///
    /// If `addr` argument yields multiple addresses, bind will be attempted with
    /// each of the addresses until one succeeds and returns the listener. If none
    /// of the addresses succeed in creating a listener, the error returned from the
    /// last attempt (the last address) is returned.
    ///
    /// # Examples
    ///
    /// Creates a MDSWP listener bound to `127.0.0.1:80`:
    ///
    /// ```rust
    /// use mdswp::MdswpListener;
    ///
    /// let listener = MdswpListener::bind("127.0.0.1:80").unwrap();
    /// ```
    ///
    /// Creates a MDSWP listener bound to `127.0.0.1:80`. If that fails, create
    /// a MDSWP listener bound to `127.0.0.1:443`:
    ///
    /// ```rust
    /// use std::net::SocketAddr;
    /// use mdswp::MdswpListener;
    ///
    /// let addrs = [
    ///     SocketAddr::from(([127, 0, 0, 1], 80)),
    ///     SocketAddr::from(([127, 0, 0, 1], 443)),
    /// ];
    /// let listener = MdswpListener::bind(&addrs[..]).unwrap();
    /// ```
    pub fn bind<A>(addr: A) -> io::Result<Arc<Self>>
    where A: ToSocketAddrs {
        let socket = UdpSocket::bind(addr)?;
        Self::__new(socket, true)
    }

    #[doc(hidden)]
    pub(crate) fn _new_for_single(peer_addr: SocketAddr) -> io::Result<Arc<Self>> {
        let local_addr = match peer_addr {
            SocketAddr::V4(..) => sock_addr_v4_any(),
            SocketAddr::V6(..) => sock_addr_v6_any()
        };
        let socket = UdpSocket::bind(local_addr)?;
        socket.connect(peer_addr)?;
        Self::__new(socket, false)
    }

    #[doc(hidden)]
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

    pub fn incoming(self: Arc<MdswpListener>) -> Incoming {
        self.into()
    }

    /// Returns if the listener has been shutdown by an unexpected error.
    pub fn is_err(&self) -> bool {
        self.error.read().unwrap().is_err()
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Accept a new incoming connection from this listener in non-blocking manner.
    /// For blocking variant see [`accept`](Self::accept) method.
    ///
    /// When established, the corresponding [`MdswpStream`] and the remote peer’s
    /// address will be returned.
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

    /// Accept a new incoming connection from this listener.
    ///
    /// This function will block the calling thread until a new MDSWP connection is
    /// established. For non-blocking variant see
    /// [`accept_nonblocking`](Self::accept_nonblocking).
    ///
    /// When established, the corresponding [`MdswpStream`] and the remote peer’s
    /// address will be returned.
    pub fn accept(&self) -> io::Result<(MdswpStream, SocketAddr)> {
        loop {
            match self.accept_nonblocking() {
                Result::Ok(Option::None) => thread::sleep(Duration::ZERO),
                Result::Ok(Option::Some(conn_tuple)) => return Result::Ok(conn_tuple),
                Result::Err(err) => return Result::Err(err)
            }
        }
    }

    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.socket.set_ttl(ttl)
    }

    /// Returns set time-to-live value.
    ///
    /// See [`set_ttl`](Self::set_ttl) for more details.
    pub fn ttl(&self) -> io::Result<u32> {
        self.socket.ttl()
    }

    #[doc(hidden)]
    pub(crate) fn _start_thread(self: &Arc<Self>) -> io::Result<()> {
        self.socket.set_nonblocking(false)?;
        let weak = Arc::downgrade(&self);
        let thread = thread::spawn(clone!(weak => || Self::__listen_thread(weak)));
        *self.thread.write().unwrap() = Option::Some(thread);
        Result::Ok(())
    }

    #[doc(hidden)]
    pub(crate) fn _add_connection(self: &Arc<Self>, peer_addr: SocketAddr, stream: Weak<StreamInner>) {
        self.connections.write().unwrap().insert(peer_addr, stream);
    }

    #[doc(hidden)]
    pub(crate) fn _remove_connection(self: &Arc<Self>, peer_addr: SocketAddr) {
        self.connections.write().unwrap().remove(&peer_addr);
    }

    #[doc(hidden)]
    pub(crate) fn _connect_listener_to(&self, addr: SocketAddr) -> io::Result<()> {
        self.socket.connect(addr)
    }

    #[doc(hidden)]
    pub(crate) fn _send_to(&self, addr: SocketAddr, segment: Segment) -> io::Result<()> {
        self.socket.send_to(&segment.into_bytes(), addr).and(Result::Ok(()))
    }

    #[doc(hidden)]
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

    #[doc(hidden)]
    fn __error(&self, err: io::Error) {
        *self.error.write().unwrap() = Result::Err(err);
    }

    #[doc(hidden)]
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

    #[doc(hidden)]
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
                    let accept_seq_num = 0; // TODO rand::thread_rng().gen();
                    let conn = StreamInner::new_by_peer(self.clone(), addr, establish_seq_num, accept_seq_num);
                    self.connections.write().unwrap().insert(addr, Arc::downgrade(&conn));
                    self.to_accept.write().unwrap().push_back(conn.clone());
                },
                _other => { let _ = self._send_to(addr, Segment::Reset); },
            }
        };
    }
}

/// An iterator that infinitely accepts connections on a [`MdswpListener`].
///
/// This struct is created by the [`MdswpListener::incoming`] method. See its
/// documentation for more.
pub struct Incoming {
    listener: Arc<MdswpListener>
}

impl From<Arc<MdswpListener>> for Incoming {
    fn from(listener: Arc<MdswpListener>) -> Self {
        Self { listener }
    }
}

impl Into<Arc<MdswpListener>> for Incoming {
    fn into(self) -> Arc<MdswpListener> {
        self.listener
    }
}

impl Iterator for Incoming {
    type Item = io::Result<(MdswpStream, SocketAddr)>;

    fn next(&mut self) -> Option<Self::Item> {
        Option::Some(self.listener.accept())
    }
}