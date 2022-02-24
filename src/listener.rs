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

use crate::MdswpStream;
use crate::segment::MAX_SEGMENT_LEN;
use crate::util::clone;

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
    connections: RwLock<HashMap<SocketAddr, Weak<MdswpStream>>>,
    to_accept: RwLock<VecDeque<Arc<MdswpStream>>>,
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
        let instance = Arc::new(Self {
            socket,
            thread: RwLock::new(Option::None),
            connections: RwLock::new(HashMap::new()),
            to_accept: RwLock::new(VecDeque::new()),
            error: RwLock::new(Result::Ok(())),
        });
        let weak = Arc::downgrade(&instance);
        let thread = thread::spawn(clone!(weak => || Self::__listen_thread(weak)));
        *instance.thread.write().unwrap() = Option::Some(thread);
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
    pub fn accept_nonblocking(&self) -> io::Result<Option<(Arc<MdswpStream>, SocketAddr)>> {
        match &*self.error.read().unwrap() {
            Result::Ok(()) => match self.to_accept.write().unwrap().pop_front() {
                Option::Some(conn) => Result::Ok(Option::Some((conn.clone(), conn.peer_addr()?))),
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
    pub fn accept(&self) -> io::Result<(Arc<MdswpStream>, SocketAddr)> {
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
    pub(crate) fn _connect_to(&self, addr: SocketAddr) -> io::Result<()> {
        self.socket.connect(addr)
    }

    #[doc(hidden)]
    pub(crate) fn _send_to(&self, addr: SocketAddr, data: &[u8]) -> io::Result<()> {
        self.socket.send_to(data, addr).and(Result::Ok(()))
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
                Result::Err(err) => this.__error(err),
                Result::Ok((n, addr)) => this.__recv(&buf[..n], addr)
            }
        }
    }

    #[doc(hidden)]
    fn __recv(self: Arc<Self>, data: &[u8], addr: SocketAddr) {
        // Try to get a connection
        let conn = match self.connections.read().unwrap().get(&addr) {
            Option::Some(weak) => weak.upgrade(),
            Option::None => Option::None
        };
        // If connection is not present yet, create a new one:
        let mut conn = match conn {
            Option::Some(conn) => conn,
            Option::None => {
                let new_conn = MdswpStream::_new(self.clone(), addr);
                self.connections.write().unwrap()
                    .insert(addr, Arc::downgrade(&new_conn));
                new_conn
            }
        };
        // Acknowledge receive of a segment
        match data.try_into() {
            Result::Ok(segment) => conn._recv_segment(segment),
            Result::Err(err) => conn._error(err)
        }
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
    type Item = io::Result<(Arc<MdswpStream>, SocketAddr)>;

    fn next(&mut self) -> Option<Self::Item> {
        Option::Some(self.listener.accept())
    }
}