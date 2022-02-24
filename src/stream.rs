//! Module for [`MdswpStream`](crate::MdswpStream) and its related data structures.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rand::Rng;

use crate::segment::Segment;
use crate::segment::SequenceNumber;
use crate::segment::SequentialSegment;
use crate::util::clone;
use crate::util::clone_io_err;
use crate::util::conn_reset_by_peer;
use crate::util::conn_reset_local;
use crate::util::conn_timeout;
use crate::util::conn_unexpected_segment;
use crate::util::sock_addr_v4_any;
use crate::util::sock_addr_v6_any;
use crate::MdswpListener;

macro_rules! listener_bind_unreachable {
    () => {
        unreachable!(
            "MdswpStream is still bound to the MdswpListener after resetting \
        a connection"
        )
    };
}

macro_rules! conn_state_unreachable {
    ($s:expr) => {
        unreachable!("This state should be unreachable now: {:?}", $s)
    };
}

/// A struct for communicating through the stream. This struct implements [`Read`] and [`Write`]
/// like the standard [`TcpStream`].
///
/// [`Read`]: std::io::Read
/// [`Write`]: std::io::Write
/// [`TcpStream`]: std::net::TcpStream
pub struct MdswpStream {
    peer_addr: SocketAddr,
    conn_state: RwLock<ConnState>,
    send_timestamps: RwLock<VecDeque<(SequenceNumber, Option<Instant>)>>,
    send_data: RwLock<BTreeMap<SequenceNumber, SequentialSegment>>,
    recv_data: RwLock<BTreeMap<SequenceNumber, SequentialSegment>>,
    listener: Arc<MdswpListener>,
    thread: RwLock<Option<JoinHandle<()>>>,
}

impl MdswpStream {
    #[doc(hidden)]
    pub(crate) fn _new(listener: Arc<MdswpListener>, peer_addr: SocketAddr) -> Arc<Self> {
        let instance = Arc::new(Self {
            peer_addr,
            conn_state: RwLock::new(ConnState::NotConnected),
            send_timestamps: RwLock::new(VecDeque::new()),
            send_data: RwLock::new(BTreeMap::new()),
            recv_data: RwLock::new(BTreeMap::new()),
            listener,
            thread: RwLock::new(Option::None),
        });
        let weak = Arc::downgrade(&instance);
        *instance.thread.write().unwrap() =
            Option::Some(thread::spawn(clone!(weak => || __thread(weak))));

        instance
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
    pub fn connect<A>(addr: A) -> io::Result<Arc<Self>>
    where
        A: ToSocketAddrs,
    {
        // Evaluate the addresses
        let addrs = addr.to_socket_addrs()?;
        // Last error:
        let mut last_err = io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "No socket address was provided",
        );
        // Try to connect to each address:
        for peer_addr in addrs {
            match Self::connect_to(peer_addr) {
                Result::Ok(conn) => return Result::Ok(conn),
                Result::Err(err) => last_err = err,
            }
        }
        // If unsuccessful, return error given from last attempt.
        Result::Err(last_err)
    }

    /// Opens a MDSWP connection to a remote host.
    ///
    /// `addr` is a [`SocketAddr`] of the remote host. Since this method does not
    /// use the [`ToSocketAddrs`] trait it is used to attempt to connect to a single
    /// [`SocketAddr`]. See [`ToSocketAddrs`] trait documentation for more details.
    ///
    /// This method tries to connect multiple times. If none of the attepts result
    /// in a successful connection, the error returned from the last connection
    /// attempt is returned.
    ///
    /// [`SocketAddr`]: std::net::SocketAddr
    /// [`ToSocketAddrs`]: std::net::ToSocketAddrs
    pub fn connect_to(addr: SocketAddr) -> io::Result<Arc<Self>> {
        let mut attept_count = 3;
        let mut last_err = Option::None;
        // Try to connect to the peer:
        while attept_count != 0 {
            match Self::__try_single_connect_to(addr) {
                Result::Ok(conn) => return Result::Ok(conn),
                Result::Err(err) => last_err = Option::Some(err),
            }
            attept_count -= 1;
        }
        // If connection was unsuccessful after few times, return error from the
        // last attempt:
        Result::Err(last_err.unwrap())
    }

    #[doc(hidden)]
    fn __try_single_connect_to(peer_addr: SocketAddr) -> io::Result<Arc<Self>> {
        // Determine local address
        let local_addr = match peer_addr {
            SocketAddr::V4(..) => sock_addr_v4_any(),
            SocketAddr::V6(..) => sock_addr_v6_any(),
        };
        // Create a listener
        let listener = MdswpListener::bind(local_addr)?;
        listener._connect_to(peer_addr)?;
        // Establish a connection
        let stream = MdswpStream::_new(listener, peer_addr);
        stream.__establish_conn();
        match stream.__get_conn_state() {
            ConnState::Connected { .. } => Result::Ok(stream),
            ConnState::Errored { error } => Result::Err(error),
            ConnState::ResetByPeer => Result::Err(conn_reset_by_peer()),
            other => unreachable!("This state should be unreachable: {:?}", other),
        }
    }


    /// Returns the socket address of the remote peer of this MDSWP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        match self.__get_conn_state() {
            ConnState::Errored { error } => Result::Err(clone_io_err(&error)),
            ConnState::ResetByPeer => Result::Err(conn_reset_by_peer()),
            ConnState::ResetLocal => Result::Err(conn_reset_local()),
            _ => Result::Ok(self.peer_addr),
        }
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Finishes the write operation to the stream.
    pub fn finish_write(&self) -> io::Result<()> {
        todo!()
    }

    /// Shuts down the connection by sending a [`Reset`](Segment::Reset) segment to
    /// the peer.
    ///
    /// Calling this function multiple times will result in an error.
    pub fn reset(&self) {
        self.__set_conn_state(ConnState::ResetLocal);
        self.send_data.write().unwrap().clear();
        self.recv_data.write().unwrap().clear();
        let _ = self.listener._send_to(self.peer_addr, &Segment::Reset.to_bytes());
    }

    /// Returns if the stream has errored.
    pub fn is_err(&self) -> bool {
        match &*self.conn_state.read().unwrap() {
            ConnState::NotConnected => false,
            ConnState::EstablishSent { .. } => false,
            ConnState::AcceptSent { .. } => false,
            ConnState::Connected { .. } => false,
            ConnState::ResetLocal => true,
            ConnState::ResetByPeer => true,
            ConnState::Errored { .. } => true,
        }
    }

    /// Returns if the stream has errored.
    pub fn err(&self) -> io::Result<()> {
        match &*self.conn_state.read().unwrap() {
            ConnState::NotConnected
            | ConnState::EstablishSent { .. }
            | ConnState::AcceptSent { .. }
            | ConnState::Connected { .. } => Result::Ok(()),
            ConnState::ResetLocal => Result::Err(conn_reset_local()),
            ConnState::ResetByPeer => Result::Err(conn_reset_by_peer()),
            ConnState::Errored { error } => {
                Result::Err(io::Error::new(error.kind(), error.to_string()))
            }
        }
    }

    /// Returns if writing has been completed.
    pub fn is_write_finished(&self) -> bool {
        todo!()
    }

    /// Returns if reading has been completed.
    pub fn is_read_finished(&self) -> bool {
        todo!()
    }

    /// Sets the TTL for the underlying UDP socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.listener.set_ttl(ttl)
    }

    /// Gets the TTL for the underlying UDP socket.
    pub fn ttl(&self) -> io::Result<u32> {
        self.listener.ttl()
    }

    #[doc(hidden)]
    pub(crate) fn _error(&self, error: io::Error) {
        self.__set_conn_state(ConnState::Errored { error });
        self.send_data.write().unwrap().clear();
        self.recv_data.write().unwrap().clear();
        let _ = self.listener._send_to(self.peer_addr, &Segment::Reset.to_bytes());
    }

    #[doc(hidden)]
    pub(crate) fn _recv_segment(&self, segment: Segment) {
        match segment {
            Segment::Reset => self.__recv_reset(),
            Segment::Establish { start_seq_num } => self.__recv_establish(start_seq_num),
            Segment::Acknowledge { seq_num } => self.__recv_acknowledge(seq_num),
            Segment::Sequential { seq_num, variant } => self.__recv_sequential(seq_num, variant),
        }
    }

    #[doc(hidden)]
    fn __recv_reset(&self) {
        *self.conn_state.write().unwrap() = ConnState::ResetByPeer;
    }

    #[doc(hidden)]
    fn __recv_establish(&self, start_seq_num: SequenceNumber) {
        match self.__get_conn_state() {
            // If we are disconnected
            ConnState::NotConnected => self.__accept_conn(start_seq_num),
            ConnState::EstablishSent { .. }
            | ConnState::AcceptSent { .. }
            | ConnState::Connected { .. } => {
                self._error(conn_unexpected_segment(&Segment::Establish {
                    start_seq_num,
                }))
            }
            ConnState::ResetLocal | ConnState::ResetByPeer | ConnState::Errored { .. } => {
                listener_bind_unreachable!()
            }
        }
    }

    #[doc(hidden)]
    fn __recv_acknowledge(&self, seq_num: SequenceNumber) {
        todo!()
    }

    #[doc(hidden)]
    fn __recv_sequential(&self, seq_num: SequenceNumber, variant: SequentialSegment) {
        self.__append_recv(seq_num, variant.clone());
        if !self.is_err() {
            match variant {
                SequentialSegment::Accept => self.__recv_accept(seq_num),
                SequentialSegment::Finish => self.__recv_finish(seq_num),
                SequentialSegment::Data { .. } => {},
            }
        }
    }

    #[doc(hidden)]
    fn __append_recv(&self, seq_num: SequenceNumber, variant: SequentialSegment) {
        self.__send_segment(Segment::Acknowledge { seq_num });
        todo!("Check for existence and evantually compare the contents")
    }

    #[doc(hidden)]
    fn __recv_accept(&self, seq_num: SequenceNumber) {
        let (local_win, peer_win) = match self.__get_conn_state() {
            // Accept segment can be received right after sending Establish:
            ConnState::EstablishSent { local_win } => (local_win, Option::None),
            // And when the Acknowledge segment was not received on first attempt,
            // e.g. connected state and same sequence number
            ConnState::Connected { local_win, peer_win, .. }
            if peer_win == seq_num => (local_win, Option::Some(peer_win)),
            // Anything else is invalid
            _ => {
                self._error(conn_unexpected_segment(&Segment::Sequential {
                    variant: SequentialSegment::Accept,
                    seq_num,
                }));
                return;
            }
        };
    }

    #[doc(hidden)]
    fn __recv_finish(&self, seq_num: SequenceNumber) {
        match self.__get_conn_state() {
            // We can receive Finish only when we are connected
            ConnState::Connected {
                local_win,
                peer_win,
                local_finish,
                peer_finish
            } => match peer_finish {
                // If Finish has been already received the sequence number of this
                // and previous Finish segment must be the same
                Option::Some(n) if n != seq_num
                => self._error(conn_unexpected_segment(&Segment::Sequential {
                    seq_num,
                    variant: SequentialSegment::Finish
                })),
                // Change state to reflect the arrival of the Finish segment
                _ => {
                    let new_conn_state = ConnState::Connected {
                        local_win,
                        peer_win,
                        local_finish,
                        peer_finish: Option::Some(seq_num)
                    };
                    self.__set_conn_state(new_conn_state);
                }
            },
            // If we are not connected it must be a mistake
            other => self._error(conn_unexpected_segment(
                &Segment::Sequential { seq_num, variant: SequentialSegment::Finish }
            )),
        }
    }

    #[doc(hidden)]
    fn __establish_conn(&self) {
        let timeout = 2 * conn_timeout();
        let start_seq_num = rand::thread_rng().gen();
        let start = Instant::now();
        self.__send_segment(Segment::Establish { start_seq_num });
        while start.elapsed() < timeout {
            match self.__get_conn_state() {
                ConnState::Connected { .. } => return,
                ConnState::ResetByPeer => return,
                ConnState::Errored { .. } => return,
                ConnState::ResetLocal => conn_state_unreachable!(ConnState::ResetLocal),
                _ => thread::sleep(Duration::ZERO),
            }
        }
    }

    #[doc(hidden)]
    fn __get_conn_state(&self) -> ConnState {
        self.conn_state.read().unwrap().clone()
    }

    #[doc(hidden)]
    fn __set_conn_state(&self, conn_state: ConnState) {
        *self.conn_state.write().unwrap() = conn_state;
    }

    #[doc(hidden)]
    fn __accept_conn(&self, peer_win: SequenceNumber) {
        let local_win = rand::thread_rng().gen();
        let new_conn_state = ConnState::AcceptSent {
            local_win,
            peer_win,
        };
        let accept_segment = Segment::Sequential {
            seq_num: local_win,
            variant: SequentialSegment::Accept,
        };

        self.__send_segment(accept_segment);
        self.__set_conn_state(new_conn_state);
    }

    #[doc(hidden)]
    fn __send_segment(&self, segment: Segment) {
        self.listener
            ._send_to(self.peer_addr, &segment.to_bytes())
            .unwrap_or_else(|error| self._error(error));
    }
}

impl Drop for MdswpStream {
    fn drop(&mut self) {
        let _ = self.reset();
    }
}

impl Read for MdswpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        todo!()
    }
}

impl Write for MdswpStream {
    /// Writes some bytes to the stream. Note that this stream is buffered and data
    /// is not sent immediately. It is always guaranteed that all data is written to
    /// the stream when calling this method.
    ///
    /// # Return value
    ///
    ///  -  [`Result::Ok`] if writing to the stream was not aborted by the
    ///     [`shutdown`] method.
    ///  -  [`Result::Err`] if either:
    ///      -  writing to the stream was ended by the [`shutdown`] method,
    ///      -  connection was reset
    ///      -  connection has errored ([`is_err`] method)
    ///
    /// [`is_err`]: MdswpStream::is_err
    /// [`shutdown`]: MdswpStream::shutdown
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        todo!()
    }

    /// Flushes the stream, e.g. forces sending all remaining bytes even if there is
    /// small amount of data to be sent.
    fn flush(&mut self) -> io::Result<()> {
        todo!()
    }
}

#[doc(hidden)]
#[derive(Debug)]
enum ConnState {
    NotConnected,
    EstablishSent {
        local_win: SequenceNumber,
    },
    AcceptSent {
        local_win: SequenceNumber,
        peer_win: SequenceNumber,
    },
    Connected {
        local_win: SequenceNumber,
        local_finish: Option<SequenceNumber>,
        peer_win: SequenceNumber,
        peer_finish: Option<SequenceNumber>,
    },
    ResetByPeer,
    ResetLocal,
    Errored {
        error: io::Error,
    },
}

impl Clone for ConnState {
    fn clone(&self) -> Self {
        match self {
            Self::NotConnected => Self::NotConnected,
            Self::EstablishSent { local_win } => Self::EstablishSent {
                local_win: *local_win,
            },
            Self::AcceptSent {
                local_win,
                peer_win,
            } => Self::AcceptSent {
                local_win: *local_win,
                peer_win: *peer_win,
            },
            Self::Connected {
                local_win,
                local_finish,
                peer_win,
                peer_finish,
            } => Self::Connected {
                local_win: *local_win,
                local_finish: *local_finish,
                peer_win: *peer_win,
                peer_finish: *peer_finish,
            },
            Self::ResetByPeer => Self::ResetByPeer,
            Self::ResetLocal => Self::ResetLocal,
            Self::Errored { error } => Self::Errored {
                error: clone_io_err(&error),
            },
        }
    }
}

#[doc(hidden)]
fn __thread(stream: Weak<MdswpStream>) {
    loop {
        let stream = match stream.upgrade() {
            Option::None => return,
            Option::Some(arc) => arc,
        };
        todo!()
    }
}
