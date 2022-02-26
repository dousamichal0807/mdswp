//! Module for [`MdswpStream`](crate::MdswpStream) and its related data structures.

use std::cmp::min;
use std::convert::TryInto;
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
use std::time::Duration;
use std::time::Instant;

use rand::Rng;

use crate::segment::MAX_DATA_LEN;
use crate::segment::Segment;
use crate::segment::SeqNumber;
use crate::segment::SeqSegment;
use crate::util::clone;
use crate::util::clone_io_err;
use crate::util::conn_reset_by_peer;
use crate::util::conn_reset_local;
use crate::util::conn_timed_out;
use crate::util::conn_timeout;
use crate::util::conn_unexpected_segment;
use crate::util::sock_addr_v4_any;
use crate::util::sock_addr_v6_any;
use crate::MdswpListener;
use crate::storage::RecvStorage;
use crate::storage::SendStorage;

/// A struct for communicating through the stream. This struct implements [`Read`] and [`Write`]
/// like the standard [`TcpStream`].
///
/// [`Read`]: std::io::Read
/// [`Write`]: std::io::Write
/// [`TcpStream`]: std::net::TcpStream
pub struct MdswpStream {
    peer_addr: SocketAddr,
    conn_state: RwLock<io::Result<()>>,
    send_buffer: RwLock<Vec<u8>>,
    send_storage: RwLock<Option<SendStorage>>,
    send_finished: RwLock<bool>,
    recv_buffer: RwLock<Vec<u8>>,
    recv_storage: RwLock<Option<RecvStorage>>,
    recv_finished: RwLock<bool>,
    listener: Arc<MdswpListener>,
    thread: RwLock<Option<JoinHandle<()>>>,
}

impl MdswpStream {
    #[doc(hidden)]
    pub(crate) fn _new(
        listener: Arc<MdswpListener>,
        peer_addr: SocketAddr,
    ) -> Arc<Self> {
        let instance = Arc::new(Self {
            peer_addr,
            conn_state: RwLock::new(Result::Ok(())),
            send_buffer: RwLock::new(Vec::new()),
            send_storage: RwLock::new(Option::None),
            send_finished: RwLock::new(false),
            recv_storage: RwLock::new(Option::None),
            recv_buffer: RwLock::new(Vec::new()),
            recv_finished: RwLock::new(false),
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
        let stream = Self::_new(listener, peer_addr);
        stream.__establish_conn();
        match stream.stream_state() {
            Result::Ok(()) => Result::Ok(stream),
            Result::Err(error) => Result::Err(error)
        }
    }


    /// Returns the socket address of the remote peer of this MDSWP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream_state()
            .and(Result::Ok(self.peer_addr))
    }

    /// Returns the socket address of the local half of this MDSWP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream_state()
            .and_then(|_| self.listener.local_addr())
    }

    /// Finishes the write operation to the stream.
    pub fn finish_write(&self) -> io::Result<()> {
        self.stream_state()?;
        match self.send_storage
            .write().unwrap()
            .as_mut().unwrap()
            .push(SeqSegment::Finish)
        {
            Result::Ok(_) => {
                *self.send_finished.write().unwrap() = true;
                Result::Ok(())
            },
            Result::Err(error) => {
                self._error(clone_io_err(&error));
                Result::Err(error)
            }
        }
    }

    /// Shuts down the connection by sending a [`Reset`](Segment::Reset) segment to
    /// the peer.
    ///
    /// Calling this function multiple times will result in an error.
    pub fn reset(&self) -> io::Result<()> {
        self.stream_state()?;
        self._error(conn_reset_local());
        Result::Ok(())
    }

    /// Returns if the stream has errored.
    pub fn is_err(&self) -> bool {
        self.conn_state.read().unwrap().is_err()
    }

    /// Returns if the stream has errored.
    pub fn stream_state(&self) -> io::Result<()> {
        match &*self.conn_state.read().unwrap() {
            Result::Ok(()) => Result::Ok(()),
            Result::Err(ref err) => Result::Err(clone_io_err(err))
        }
    }

    /// Returns if writing has been completed.
    pub fn is_write_finished(&self) -> io::Result<bool> {
        self.stream_state()
            .map(|_| self.send_storage.read().unwrap()
                .as_ref().unwrap().finished())
    }

    /// Returns if reading has been completed.
    pub fn is_read_finished(&self) -> io::Result<bool> {
        self.stream_state()
            .map(|_| *self.recv_finished.read().unwrap())
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
        *self.conn_state.write().unwrap() = Result::Err(error);
        *self.send_storage.write().unwrap() = Option::None;
        *self.recv_storage.write().unwrap() = Option::None;
        let _ = self.listener._send_to(self.peer_addr, &Segment::Reset.to_bytes());
    }

    #[doc(hidden)]
    pub(crate) fn _recv_segment(&self, segment: Segment) {
        match segment {
            Segment::Reset => self._error(conn_reset_by_peer()),
            Segment::Establish { start_seq_num } => self.__recv_establish(start_seq_num),
            Segment::Acknowledge { seq_num } => self.__recv_acknowledge(seq_num),
            Segment::Sequential { seq_num, variant } => self.__recv_sequential(seq_num, variant),
        }
    }

    #[doc(hidden)]
    fn __recv_establish(&self, peer_win: SeqNumber) {
        if self.send_storage.read().unwrap().is_some() {
            self._error(conn_unexpected_segment(
                &Segment::Establish { start_seq_num: peer_win }));
            return;
        } else {
            let local_win = rand::thread_rng().gen();
            *self.send_storage.write().unwrap() = Option::Some(SendStorage::new(local_win));
            *self.recv_storage.write().unwrap() = Option::Some(RecvStorage::new_by_peer(peer_win));
        }
    }

    #[doc(hidden)]
    fn __recv_acknowledge(&self, seq_num: SeqNumber) {
        let mut send_storage = self.send_storage.write().unwrap();
        match send_storage.as_mut() {
            Option::None => self._error(conn_unexpected_segment(
                &Segment::Acknowledge { seq_num })),
            Option::Some(send_storage) => match send_storage.acknowledge(seq_num) {
                Result::Err(error) => self._error(error),
                Result::Ok(()) => {}
            }
        }
    }

    #[doc(hidden)]
    fn __recv_sequential(&self, seq_num: SeqNumber, variant: SeqSegment) {
        match self.recv_storage.write().unwrap().as_mut() {
            Option::None => self._error(conn_unexpected_segment(
                &Segment::Sequential { seq_num, variant })),
            Option::Some(recv_storage) => match recv_storage.recv(seq_num, variant) {
                Result::Err(error) => self._error(error),
                Result::Ok(()) => {}
            }
        }
    }

    #[doc(hidden)]
    fn __establish_conn(&self) {
        let timeout = 2 * conn_timeout();
        let start_seq_num = rand::thread_rng().gen();
        *self.send_storage.write().unwrap() = Option::Some(SendStorage::new(start_seq_num));
        *self.recv_storage.write().unwrap() = Option::Some(RecvStorage::new_by_peer(start_seq_num));
        let start = Instant::now();
        self.__send_segment(Segment::Establish { start_seq_num });
        while start.elapsed() < timeout {
            match self.stream_state() {
                Result::Err(err) => { self._error(err); return; },
                Result::Ok(()) => {}
            };
            let segment = match self.recv_storage.write().unwrap().as_mut().unwrap().pop() {
                Option::None => { thread::sleep(Duration::ZERO); continue; }
                Option::Some(segment) => segment
            };
            match segment {
                SeqSegment::Accept => return,
                other => {
                    self._error(conn_unexpected_segment(format!("{:?}", other)));
                    return;
                }
            }
        }
        self._error(conn_timed_out())
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
        let mut stream_buf = self.recv_buffer.write().unwrap();
        loop {
            match self.recv_storage.write().unwrap()
                .as_mut().unwrap().pop()
            {
                Option::None => thread::sleep(Duration::ZERO),
                Option::Some(segment) => match segment {
                    SeqSegment::Finish => {
                        *self.recv_finished.write().unwrap() = true;
                        break;
                    },
                    SeqSegment::Data { data } => {
                        stream_buf.append(&mut data.into_iter().collect());
                        break;
                    }
                    SeqSegment::Accept => unreachable!("ACCEPT segment cannot be popped"),
                }
            }
        }
        let len = min(stream_buf.len(), buf.len());
        let data: Vec<u8> = stream_buf.drain(..len).collect();
        buf.copy_from_slice(&data);
        Result::Ok(len)
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
        self.stream_state()?;
        let mut send_buf = self.send_buffer.write().unwrap();
        send_buf.write_all(buf)?;
        while send_buf.len() >= MAX_DATA_LEN {
            let bytes: Vec<u8> = send_buf.drain(..MAX_DATA_LEN).into_iter().collect();
            let data = bytes[..].try_into().unwrap();
            self.send_storage.write().unwrap()
                .as_mut().unwrap()
                .push(SeqSegment::Data { data })?;
        }
        Result::Ok(buf.len())
    }

    /// Flushes the stream, e.g. forces sending all remaining bytes even if there is
    /// small amount of data to be sent.
    fn flush(&mut self) -> io::Result<()> {
        self.stream_state()?;
        let mut send_buf = self.send_buffer.write().unwrap();
        while send_buf.len() > 0 {
            let n = min(MAX_DATA_LEN, send_buf.len());
            let bytes: Vec<u8> = send_buf.drain(..n).into_iter().collect();
            let data = bytes[..].try_into().unwrap();
            self.send_storage.write().unwrap()
                .as_mut().unwrap()
                .push(SeqSegment::Data { data })?;
        }
        Result::Ok(())
    }
}

#[doc(hidden)]
fn __thread(stream: Weak<MdswpStream>) {
    loop {
        let stream = match stream.upgrade() {
            Option::None => return,
            Option::Some(arc) => arc,
        };
        // Send next acknowledge if there is something to acknowledge:
        let next_ack = stream.recv_storage.write().unwrap()
            .as_mut().unwrap()
            .pop_acknowledge();
        if let Option::Some(seq_num) = next_ack {
            let segment = Segment::Acknowledge { seq_num };
            match stream.listener._send_to(stream.peer_addr, &segment.to_bytes()) {
                Result::Err(error) => { stream._error(error); return; }
                Result::Ok(_) => {}
            }
        }
        // Send sequential segments if tey are avaliable
        let mut send_storage = stream.send_storage.write().unwrap();
        loop {
            match send_storage.as_mut().unwrap().pop() {
                Option::None => break,
                Option::Some((seq_num, variant)) => {
                    let segment = Segment::Sequential { seq_num, variant };
                    if let Result::Err(error) = stream.listener._send_to(stream.peer_addr, &segment.to_bytes()) {
                        stream._error(error);
                        return;
                    }
                }
            }
        }
    }
}