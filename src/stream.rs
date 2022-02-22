//! Module for [`MdswpStream`](crate::MdswpStream) and its related data structures.

use crate::util::clone;
use crate::util::conn_invalid_ack_segment;
use crate::util::conn_reset_by_peer;
use crate::util::conn_unexpected_segment;
use crate::util::conn_write_finished;
use crate::util::CONNECTION_TIMEOUT;
use crate::util::SOCKADDR_V4_ANY;
use crate::util::SOCKADDR_V6_ANY;
use crate::segment::MAX_DATA_LEN;
use crate::segment::MAX_SEGMENT_LEN;
use crate::segment::Segment;
use crate::segment::SequenceNumber;
use crate::segment::SequentialSegment;

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::convert::TryInto;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::RwLock;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use rand::Rng;

#[doc(hidden)]
const WINDOW_SIZE: SequenceNumber = 5;

/// A struct for communicating through the stream. This struct implements [`Read`] and [`Write`]
/// like the standard [`TcpStream`].
///
/// [`Read`]: std::io::Read
/// [`Write`]: std::io::Write
/// [`TcpStream`]: std::net::TcpStream
pub struct MdswpStream {
    socket: UdpSocket,
    peer_addr: SocketAddr,
    send_storage: RwLock<SendDataStorage>,
    recv_storage: RwLock<RecvDataStorage>,
    error: RwLock<io::Result<()>>,
    send_thr: RwLock<Option<JoinHandle<()>>>,
    recv_thr: RwLock<Option<JoinHandle<()>>>,
}

impl MdswpStream {
    #[doc(hidden)]
    pub(crate) fn _new(
        socket: UdpSocket,
        peer_addr: SocketAddr,
        local_win_start: SequenceNumber,
        peer_win_start: SequenceNumber
    ) -> Arc<Self> {
        let instance = Arc::new(Self {
            socket,
            peer_addr,
            send_storage: RwLock::new(SendDataStorage::new(local_win_start)),
            recv_storage: RwLock::new(RecvDataStorage::new(peer_win_start)),
            error: RwLock::new(Result::Ok(())),
            send_thr: RwLock::new(Option::None),
            recv_thr: RwLock::new(Option::None),
        });
        *instance.send_thr.write().unwrap() = Option::Some(thread::spawn(clone!(instance => || instance.__send_thread())));
        *instance.recv_thr.write().unwrap() = Option::Some(thread::spawn(clone!(instance => || instance.__recv_thread())));

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
            A: ToSocketAddrs
    {
        // Evaluate the addresses
        let addrs = addr.to_socket_addrs()?;
        // Last error:
        let mut last_err = io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "No socket address was provided"
        );
        // Try to connect to each address:
        for peer_addr in addrs {
            match Self::connect_to(peer_addr) {
                Result::Ok(conn) => return Result::Ok(conn),
                Result::Err(err) => last_err = err
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
                Result::Err(err) => last_err = Option::Some(err)
            }
            attept_count -= 1;
        }
        // If connection was unsuccessful after few times, return error from the
        // last attempt:
        Result::Err(last_err.unwrap())
    }

    #[doc(hidden)]
    fn __try_single_connect_to(peer_addr: SocketAddr) -> io::Result<Arc<Self>> {
        let mut buf = [0; MAX_SEGMENT_LEN];
        // Assign unspecified local address according to the peer IP address version
        let local_addr = match peer_addr {
            SocketAddr::V4(..) => SOCKADDR_V4_ANY.clone(),
            SocketAddr::V6(..) => SOCKADDR_V6_ANY.clone()
        };
        // Generate random number where local window should start:
        let local_win_start = rand::thread_rng().gen();
        // Try to create a new UdpSocket:
        let socket = UdpSocket::bind(local_addr)?;
        socket.connect(peer_addr)?;
        socket.set_broadcast(false)?;
        socket.set_read_timeout(Option::Some(CONNECTION_TIMEOUT.clone()))?;
        socket.set_write_timeout(Option::Some(CONNECTION_TIMEOUT.clone()))?;
        // Send ESTABLISH and wait for response
        let establish_segment = Segment::Establish { start_seq_num: local_win_start };
        socket.send(&establish_segment.to_bytes())?;
        let recv_len = socket.recv(&mut buf)?;
        let segment = buf[..recv_len].try_into()?;
        // Return value
        match segment {
            Segment::Reset => Result::Err(conn_reset_by_peer!()),
            Segment::Sequential {
                seq_num: peer_win_start,
                variant: SequentialSegment::Accept
            } => Result::Ok(Self::_new(socket, peer_addr, local_win_start, peer_win_start)),
            other => {
                socket.send(&Segment::Reset.to_bytes())?;
                Result::Err(conn_unexpected_segment!(other))
            }
        }
    }

    /// Returns the socket address of the remote peer of this MDSWP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        let err = self.error.read().unwrap();
        match &*err {
            Result::Ok(()) => Result::Ok(self.peer_addr),
            Result::Err(err) => Result::Err(io::Error::new(err.kind(), err.to_string()))
        }
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Finishes the write operation to the stream.
    pub fn finish_write(&self) -> io::Result<()> {
        self.send_storage.write().unwrap().finish()
    }

    /// Shuts down the connection by sending a [`Reset`](Segment::Reset) segment to
    /// the peer.
    ///
    /// Calling this function multiple times will result in an error.
    pub fn reset(&mut self) -> io::Result<()> {
        let mut err = self.error.write().unwrap();
        match &*err {
            Result::Ok(()) => {
                *err = Result::Err(conn_reset_by_peer!());
                Result::Ok(())
            },
            Result::Err(err) => Result::Err(io::Error::new(err.kind(), err.to_string()))
        }
    }

    /// Returns if the stream has errored.
    pub fn is_err(&self) -> bool {
        self.error.read().unwrap().is_err()
    }

    /// Returns if writing has been completed.
    pub fn is_write_finished(&self) -> bool {
        self.send_storage.read().unwrap().has_finished()
    }

    /// Returns if reading has been completed.
    pub fn is_read_finished(&self) -> bool {
        self.recv_storage.read().unwrap().has_finished()
    }

    /// Sets the TTL for the underlying UDP socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.socket.set_ttl(ttl)
    }

    /// Gets the TTL for the underlying UDP socket.
    pub fn ttl(&self) -> io::Result<u32> {
        self.socket.ttl()
    }

    #[doc(hidden)]
    fn __error(&self, err: io::Error) {
        *self.error.write().unwrap() = Result::Err(err);
    }

    #[doc(hidden)]
    fn __next_send_segment(&self) -> Option<Segment> {
        todo!()
    }

    #[doc(hidden)]
    fn __recv_segment(&self, segment: Segment) {
        match segment {
            Segment::Reset => self.__error(conn_reset_by_peer!()),
            Segment::Establish { .. } => todo!("Toto je chyba"),
            Segment::Acknowledge { seq_num } => {
                let mut send = self.send_storage.write().unwrap();
                match send.register_acknowledge(seq_num) {
                    Result::Ok(()) => {}
                    Result::Err(err) => self.__error(err)
                }
            },
            Segment::Sequential { seq_num, variant } => {
                let mut recv = self.recv_storage.write().unwrap();
                match recv.recv_segment(seq_num, variant) {
                    Result::Ok(()) => {},
                    Result::Err(err) => self.__error(err)
                }
            }
        }
    }

    #[doc(hidden)]
    fn __send_thread(self: Arc<Self>) {
        while !self.is_err() {
            match self.__next_send_segment() {
                Option::Some(segment) => {
                    let bytes = segment.to_bytes();
                    match self.socket.send_to(&bytes, self.peer_addr) {
                        Result::Ok(n) => assert_eq!(n, bytes.len()),
                        Result::Err(ref err) if err.kind() == io::ErrorKind::Interrupted => {},
                        Result::Err(err) => self.__error(err),
                    }
                }
                Option::None => thread::sleep(Duration::ZERO)
            }
        }
    }

    #[doc(hidden)]
    fn __recv_thread(self: Arc<Self>) {
        let mut buf = [0; u16::MAX as usize];

        while !self.error.read().unwrap().is_err() {
            let len = match self.socket.recv(&mut buf) {
                Result::Ok(len) => len,
                Result::Err(err) =>
                    if err.kind() == io::ErrorKind::Interrupted { continue }
                    else { self.__error(err); return; }
            };

            match buf[0..len].try_into() {
                Result::Ok(segment) => self.__recv_segment(segment),
                Result::Err(err) => self.__error(conn_unexpected_segment!(err))
            };


        }
    }
}

impl Drop for MdswpStream {
    fn drop(&mut self) {
        let _ = self.reset();
        self.send_thr.write().unwrap().take().unwrap().join().unwrap();
        self.recv_thr.write().unwrap().take().unwrap().join().unwrap();
    }
}

impl Read for MdswpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.recv_storage.write().unwrap().read(buf)
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
        self.send_storage.write().unwrap().write(buf)
    }

    /// Flushes the stream, e.g. forces sending all remaining bytes even if there is
    /// small amount of data to be sent.
    fn flush(&mut self) -> io::Result<()> {
        self.send_storage.write().unwrap().flush()
    }
}

#[doc(hidden)]
struct SendDataStorage {
    buffer: Vec<u8>,
    window: SequenceNumber,
    data: VecDeque<(SequentialSegment, Option<Instant>)>,
}

impl SendDataStorage {
    /// Creates a new instance of [`SendDataStorage`].
    pub fn new(window_start: SequenceNumber) -> Self {
        Self {
            buffer: Vec::new(),
            window: window_start,
            data: VecDeque::new(),
        }
    }

    /// Puts a [`Finish`] segment into the queue. After calling this function, cannot
    /// send more data to peer.
    ///
    /// # Return value
    ///
    /// - [`Result::Ok`] if writing to the stream was not finished yet
    /// - [`Result::Err`] otherwise
    pub fn finish(&mut self) -> io::Result<()> {
        if self.has_finished() {
            Result::Err(conn_write_finished!())
        } else {
            self.__flush_unchecked();
            self.data.push_back((SequentialSegment::Finish, Option::None));
            Result::Ok(())
        }
    }

    /// Registers the acknowledgement and slides the window.
    pub fn register_acknowledge(&mut self, seq_num: SequenceNumber) -> io::Result<()> {
        // Index in the queue
        let index = seq_num.wrapping_sub(self.window);
        let index_usize = index as usize;
        // If index is out of range
        if index >= WINDOW_SIZE || index_usize >= self.data.len() {
            return Result::Err(conn_invalid_ack_segment!());
        }
        // Slide the window:
        self.window = self.window.wrapping_add(index);
        // Drain data that will not be sent again
        self.data.drain(0..=index_usize);
        Result::Ok(())
    }

    pub fn update_send_time(&mut self, seq_num: u8) -> io::Result<()> {
        // Index in the queue
        let index = seq_num.wrapping_sub(self.window);
        let index_usize = index as usize;
        // If index is out of range
        if index >= WINDOW_SIZE || index_usize >= self.data.len() {
            return Result::Err(conn_invalid_ack_segment!());
        }
        // Update send time:
        self.data[index_usize].1 = Option::Some(Instant::now());
        Result::Ok(())
    }

    /// Returns if the sending data has ended.
    pub fn has_finished(&self) -> bool {
        if self.data.is_empty() {
            return false;
        }
        let last_segment = &self.data[self.data.len() - 1];
        match last_segment.0 {
            SequentialSegment::Finish => true,
            SequentialSegment::Data { .. } => false,
            SequentialSegment::Accept => unreachable!("Accept segment cannot occur here"),
        }
    }

    fn __flush_unchecked(&mut self) {
        let data = (&self.buffer[..]).try_into().unwrap();
        self.data.push_back((SequentialSegment::Data { data }, Option::None));
    }
}

impl Write for SendDataStorage {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.has_finished() {
            Result::Err(conn_write_finished!())
        } else {
            self.buffer.extend(buf.into_iter());
            while self.buffer.len() >= MAX_DATA_LEN {
                let data = &self.buffer[0..MAX_DATA_LEN];
                let data = data.try_into().unwrap();
                self.data.push_back((SequentialSegment::Data { data }, Option::None));
            }
            Result::Ok(buf.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.has_finished() {
            Result::Err(conn_write_finished!())
        } else {
            self.__flush_unchecked();
            Result::Ok(())
        }
    }
}

#[doc(hidden)]
struct RecvDataStorage {
    data: BTreeMap<SequenceNumber, SequentialSegment>,
    window: (SequenceNumber, SequenceNumber),
}

impl RecvDataStorage {
    pub fn new(window_start: SequenceNumber) -> Self {
        let window_end = window_start.wrapping_add(WINDOW_SIZE);
        Self {
            data: BTreeMap::new(),
            window: (window_start, window_end),
        }
    }

    pub fn recv_segment(&mut self, seq_num: SequenceNumber, segment: SequentialSegment) -> io::Result<()> {
        let (window_start, _) = self.window;
        let diff = seq_num.overflowing_sub(window_start).0;

        if diff >= WINDOW_SIZE {
            return Result::Err(conn_invalid_ack_segment!());
        }

        if let Option::Some(curr_seg) = self.data.get(&seq_num) {
            return if *curr_seg != segment {
                Result::Err(conn_unexpected_segment!(
                    "Segment sent more than once, last time with different content"
                ))
            } else {
                Result::Ok(())
            };
        }

        self.data.insert(seq_num, segment);
        Result::Ok(())
    }

    pub fn has_finished(&self) -> bool {
        todo!()
    }
}

impl Read for RecvDataStorage {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        todo!()
    }
}
