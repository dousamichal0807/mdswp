use crate::storage::StreamDataStorage;
use crate::threading::send_thread;
use crate::threading::recv_thread;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown};
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::net::UdpSocket;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

/// A struct for communicating through the stream. This struct implements [`Read`] and [`Write`]
/// like the standard [`TcpStream`].
///
/// [`Read`]: std::io::Read
/// [`Write`]: std::io::Write
/// [`TcpStream`]: std::net::TcpStream
pub struct MdswpStream {
    data_storage: Arc<StreamDataStorage>,
    send_thr: Option<JoinHandle<()>>,
    recv_thr: Option<JoinHandle<()>>,
}

impl MdswpStream {
    pub(crate) fn _new(socket: UdpSocket) -> io::Result<MdswpStream>
    {
        let data = Arc::new(StreamDataStorage::new(socket));
        let send_thr = thread::spawn(clone!(data => move || send_thread(data)));
        let recv_thr = thread::spawn(clone!(data => move || recv_thread(data)));

        Result::Ok(Self {
            data_storage: data,
            send_thr: Option::Some(send_thr),
            recv_thr: Option::Some(recv_thr),
        })
    }

    /// Opens a MDSWP connection to a remote host.
    ///
    /// `addr` is an address of the remote host. Anything which implements [`ToSocketAddrs`] trait
    /// can be supplied for the address; see this trait documentation for concrete examples.
    ///
    /// If `addr` yields multiple addresses, connect will be attempted with each of the addresses
    /// until a connection is successful. If none of the addresses result in a successful
    /// connection, the error returned from the last connection attempt (the last address) is
    /// returned.
    ///
    /// [`ToSocketAddrs`]: std::net::ToSocketAddrs
    pub fn connect<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs
    {
        let addrs = addr.to_socket_addrs()?;
        let mut socket = Result::Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Given argument returned no address to connect to"
        ));

        for peer_addr in addrs {
            // Assign unspecified IPv4/IPv6 address according to the peer address version:
            let local_addr = match peer_addr {
                SocketAddr::V4(..) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
                SocketAddr::V6(..) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
            };
            // Try to create a new UdpSocket:
            socket = socket.or({
                let socket = UdpSocket::bind(local_addr)?;
                socket.connect(peer_addr)
                      .and(Result::Ok(socket))
            });
            // Get out of the loop if connection is successful:
            if socket.is_ok() { break }
        }

        let socket = socket?;

        todo!("Send ESTABLISH and wait for ACCEPT");
        Self::_new(socket)
    }

    /// Opens a TCP connection to a remote host with a timeout.
    ///
    /// Unlike [`connect`], [`connect_timeout`] takes a single [`SocketAddr`] since timeout must be
    /// applied to individual addresses.
    ///
    /// It is an error to pass a zero [`Duration`] to this function.
    pub fn connect_timeout(addr: &SocketAddr) -> io::Result<Self> {
        todo!("Send ESTABLISH and wait for ACCEPT")
        // Self::_new(local, peer)
    }

    /// Returns the socket address of the remote peer of this MDSWP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.data_storage.socket().peer_addr()
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.data_storage.socket().local_addr()
    }

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O on the specified portions to return
    /// immediately with an appropriate value (see the documentation of [`Shutdown`]).
    ///
    /// Calling this function multiple times will result in an error.
    pub fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        self.data_storage.shutdown(how)
    }

    /// Creates a new independently owned handle to the stream.
    ///
    /// The returned [`MdswpStream`] is a reference to the same stream that this object references.
    /// Both handles will read and write the same stream of data, and options set on one stream will
    /// be propagated to the other stream.
    pub fn try_clone(&self) -> io::Result<Self> {
        todo!()
    }

    /// Sets the read timeout to the timeout specified.
    ///
    /// If the value specified is [`Option::None`], then read calls will block indefinitely.
    /// [`Result::Err`] is returned if the zero [`Duration`] is passed to this method.
    /// Platform-specific behavior
    ///
    /// When read time is exceeded, read call will return [`ErrorKind::TimedOut`].
    ///
    /// [`ErrorKind::TimedOut`]: std::io::ErrorKind::TimedOut
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        todo!()
    }

    /// Returns the read timeout. If the timeout is [`Option::None`], then read calls will block
    /// indefinitely.
    pub fn read_timeout(&mut self) -> io::Result<Option<Duration>> {
        todo!()
    }

    /// Sets the TTL for the underlying UDP socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.data_storage.socket().set_ttl(ttl)
    }

    /// Gets the TTL for the underlying UDP socket.
    pub fn ttl(&self) -> io::Result<u32> {
        self.data_storage.socket().ttl()
    }
}

impl Read for MdswpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data_storage.read(buf)
    }
}

impl Write for MdswpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data_storage.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.data_storage.flush()
    }
}