use std::cmp::min;
use std::convert::TryInto;
use std::io;
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
use crate::util::{clone, conn_read_finished, conn_unexp_establish};
use crate::util::clone_io_err;
use crate::util::conn_reset_by_peer;
use crate::util::conn_reset_local;
use crate::util::conn_timed_out;
use crate::util::conn_timeout;
use crate::util::conn_unexp_seg;
use crate::MdswpListener;

use super::recv::RecvStorage;
use super::send::SendStorage;

pub(crate) struct StreamInner {
    peer_addr: SocketAddr,
    conn_state: RwLock<io::Result<()>>,
    send_buffer: RwLock<Vec<u8>>,
    send_storage: RwLock<SendStorage>,
    send_finished: RwLock<bool>,
    recv_buffer: RwLock<Vec<u8>>,
    recv_storage: RwLock<RecvStorage>,
    recv_finished: RwLock<bool>,
    listener: Arc<MdswpListener>,
    thread: RwLock<Option<JoinHandle<()>>>,
}

impl StreamInner {
    pub(crate) fn new_by_peer(
        listener: Arc<MdswpListener>,
        peer_addr: SocketAddr,
        establish_seq_num: SeqNumber,
        accept_seq_num: SeqNumber
    ) -> Arc<Self> {
        let instance = Arc::new(Self {
            peer_addr,
            conn_state: RwLock::new(Result::Ok(())),
            send_buffer: RwLock::new(Vec::new()),
            send_storage: RwLock::new(SendStorage::new_by_peer(accept_seq_num)),
            send_finished: RwLock::new(false),
            recv_storage: RwLock::new(RecvStorage::new_by_peer(establish_seq_num)),
            recv_buffer: RwLock::new(Vec::new()),
            recv_finished: RwLock::new(false),
            listener,
            thread: RwLock::new(Option::None),
        });
        let weak = Arc::downgrade(&instance);
        *instance.thread.write().unwrap() = Option::Some(thread::spawn(clone!(weak => || __thread(weak))));
        instance
    }

    pub(crate) fn new_by_local(
        listener: Arc<MdswpListener>,
        peer_addr: SocketAddr,
        establish_seq_num: SeqNumber,
        accept_seq_num: SeqNumber
    ) -> Arc<Self> {
        let instance = Arc::new(Self {
            peer_addr,
            conn_state: RwLock::new(Result::Ok(())),
            send_buffer: RwLock::new(Vec::new()),
            send_storage: RwLock::new(SendStorage::new_by_local(establish_seq_num)),
            send_finished: RwLock::new(false),
            recv_storage: RwLock::new(RecvStorage::new_by_local(accept_seq_num)),
            recv_buffer: RwLock::new(Vec::new()),
            recv_finished: RwLock::new(false),
            listener,
            thread: RwLock::new(Option::None),
        });
        let weak = Arc::downgrade(&instance);
        *instance.thread.write().unwrap() = Option::Some(thread::spawn(clone!(weak => || __thread(weak))));
        instance
    }

    pub(crate) fn connect<A>(addr: A) -> io::Result<Arc<Self>>
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

    pub(crate) fn connect_to(addr: SocketAddr) -> io::Result<Arc<Self>> {
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

    fn __try_single_connect_to(peer_addr: SocketAddr) -> io::Result<Arc<Self>> {
        // Establish segment
        let establish_seq_num = 128; // TODO rand::thread_rng().gen();
        let establish_seg = Segment::Establish { start_seq_num: establish_seq_num };
        // Create a listener
        let listener = MdswpListener::_new_for_single(peer_addr)?;
        listener._send_to(peer_addr, establish_seg)?;
        let start = Instant::now();
        // Wait for accept:
        while start.elapsed() < conn_timeout() {
            // Try get segment:
            let recv_seg = listener._try_recv();
            match recv_seg {
                Result::Err(err) => return Result::Err(err),
                Result::Ok(Option::None) => thread::sleep(Duration::ZERO),
                Result::Ok(Option::Some(recv_seg)) => return match recv_seg {
                    Segment::Reset => Result::Err(conn_reset_by_peer()),
                    Segment::Sequential {
                        seq_num: accept_seq_num,
                        variant: SeqSegment::Accept
                    } => {
                        let stream = StreamInner::new_by_local(
                            listener.clone(), peer_addr, establish_seq_num, accept_seq_num);
                        listener._start_thread()?;
                        listener._add_connection(peer_addr, Arc::downgrade(&stream));
                        Result::Ok(stream)
                    },
                    other => {
                        let _ = listener._send_to(peer_addr, Segment::Reset);
                        Result::Err(conn_unexp_seg(format!("{} was sent instead of ACCEPT", other)))
                    }
                }
            }
        };
        Result::Err(conn_timed_out())
    }

    pub(crate) fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream_state()
            .and(Result::Ok(self.peer_addr))
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream_state()
            .and_then(|_| self.listener.local_addr())
    }

    pub(crate) fn write(&self, buf: &[u8]) -> io::Result<usize> {
        self.stream_state()?;
        let mut send_buf = self.send_buffer.write().unwrap();
        send_buf.write_all(buf)?;
        while send_buf.len() >= MAX_DATA_LEN {
            let bytes: Vec<u8> = send_buf.drain(..MAX_DATA_LEN).into_iter().collect();
            let data = bytes[..].try_into().unwrap();
            self.send_storage.write().unwrap().push(SeqSegment::Data { data })?;
        }
        Result::Ok(buf.len())
    }

    pub(crate) fn flush(&self) -> io::Result<()> {
        self.stream_state()?;
        let mut send_buf = self.send_buffer.write().unwrap();
        while send_buf.len() > 0 {
            let n = min(MAX_DATA_LEN, send_buf.len());
            let bytes: Vec<u8> = send_buf.drain(..n).into_iter().collect();
            let data = bytes[..].try_into().unwrap();
            self.send_storage.write().unwrap().push(SeqSegment::Data { data })?;
        }
        Result::Ok(())
    }

    pub(crate) fn finish_write(&self) -> io::Result<()> {
        self.stream_state()?;
        let mut send_storage = self.send_storage.write().unwrap();
        let result = send_storage.push(SeqSegment::Finish);
        drop(send_storage);
        match result {
            Result::Ok(_) => {
                loop {
                    // This is required to hold lock for the stortest possible time
                    let send_storage = self.send_storage.read().unwrap();
                    let sleep = !send_storage.is_empty();
                    drop(send_storage);
                    if sleep { thread::sleep(Duration::ZERO) }
                    else { break; }
                }
                *self.send_finished.write().unwrap() = true;
                Result::Ok(())
            },
            Result::Err(error) => {
                self.error(clone_io_err(&error), true);
                Result::Err(error)
            }
        }
    }

    pub(crate) fn reset(&self) -> io::Result<()> {
        self.stream_state()?;
        self.error(conn_reset_local(), true);
        Result::Ok(())
    }

    pub(crate) fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut stream_buf = self.recv_buffer.write().unwrap();
        if self.is_read_finished()? {
            return Result::Err(conn_read_finished());
        }
        while stream_buf.is_empty() && !self.is_read_finished()? {
            let mut recv_storage = self.recv_storage.write().unwrap();
            let segment = recv_storage.pop();
            drop(recv_storage);
            match segment {
                Option::None => thread::sleep(Duration::ZERO),
                Option::Some(segment) => match segment {
                    SeqSegment::Finish => {
                        println!("FINISH!!!");
                        *self.recv_finished.write().unwrap() = true
                    },
                    SeqSegment::Data { data } => stream_buf.append(&mut data.into_iter().collect()),
                    SeqSegment::Accept => println!("WARNING! ACCEPT segment cannot be popped"),
                }
            }
        }
        let len = min(stream_buf.len(), buf.len());
        let data: Vec<u8> = stream_buf.drain(..len).collect();
        buf[..len].copy_from_slice(&data);
        Result::Ok(len)
    }

    pub(crate) fn is_err(&self) -> bool {
        self.conn_state.read().unwrap().is_err()
    }

    pub(crate) fn stream_state(&self) -> io::Result<()> {
        match &*self.conn_state.read().unwrap() {
            Result::Ok(()) => Result::Ok(()),
            Result::Err(ref err) => Result::Err(clone_io_err(err))
        }
    }

    pub(crate) fn is_write_finished(&self) -> io::Result<bool> {
        self.stream_state()
            .map(|_| *self.send_finished.read().unwrap())
    }

    pub(crate) fn is_read_finished(&self) -> io::Result<bool> {
        self.stream_state()
            .map(|_| *self.recv_finished.read().unwrap())
    }

    /// Sets the TTL for the underlying UDP socket.
    pub(crate) fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.listener.set_ttl(ttl)
    }

    /// Gets the TTL for the underlying UDP socket.
    pub(crate) fn ttl(&self) -> io::Result<u32> {
        self.listener.ttl()
    }

    pub(crate) fn error(&self, error: io::Error, send_reset: bool) {
        // Ignore further errors
        if self.is_err() { return; }
        // Acquire all read locks
        let mut conn_state = self.conn_state.write().unwrap();
        let mut send_storage = self.send_storage.write().unwrap();
        let mut recv_storage = self.recv_storage.write().unwrap();
        // Change values
        self.listener._remove_connection(self.peer_addr);
        *conn_state = Result::Err(error);
        send_storage.clear();
        recv_storage.clear();
        // Release locks
        drop(conn_state);
        drop(send_storage);
        drop(recv_storage);
        // Wait for thread now:
        //let mut thread = self.thread.write().unwrap();
        //if let Option::Some(thr) = thread.take() { let _ = thr.join(); }
        //drop(thread);
        // Send reset:
        if send_reset {
            let _ = self.listener._send_to(self.peer_addr, Segment::Reset);
        }
    }

    pub(crate) fn recv_segment(&self, segment: Segment) {
        match segment {
            Segment::Reset => self.error(conn_reset_by_peer(), false),
            Segment::Establish { .. } => self.error(conn_unexp_establish(), true),
            Segment::Acknowledge { seq_num } => self.__recv_acknowledge(seq_num),
            Segment::Sequential { seq_num, variant } => self.__recv_sequential(seq_num, variant),
        }
    }

    fn __recv_acknowledge(&self, seq_num: SeqNumber) {
        let mut send_storage = self.send_storage.write().unwrap();
        match send_storage.acknowledge(seq_num) {
            Result::Err(error) => {
                drop(send_storage);
                self.error(error, true); },
            Result::Ok(()) => {}
        }
    }

    fn __recv_sequential(&self, seq_num: SeqNumber, variant: SeqSegment) {
        let mut recv_storage = self.recv_storage.write().unwrap();
        let result = recv_storage.recv(seq_num, variant);
        drop(recv_storage);
        match result {
            Result::Err(error) => self.error(error, true),
            Result::Ok(()) => {}
        }
    }

    fn __send_segment(&self, segment: Segment) {
        self.listener
            ._send_to(self.peer_addr, segment)
            .unwrap_or_else(|error| self.error(error, true));
    }
}

impl Drop for StreamInner {
    fn drop(&mut self) {
        if !self.is_err() {
            if !self.is_write_finished().unwrap() || !self.is_read_finished().unwrap() {
                let _ = self.reset();
            }
        }
    }
}


fn __thread(stream: Weak<StreamInner>) {
    loop {
        let stream = match stream.upgrade() {
            Option::None => return,
            Option::Some(ref stream) if stream.is_err() => return,
            Option::Some(ref stream) if stream.is_write_finished().unwrap() => return,
            Option::Some(stream) => stream,
        };
        // Get next acknowledgement segment:
        let mut recv_storage = stream.recv_storage.write().unwrap();
        let next_ack = recv_storage.pop_acknowledge();
        drop(recv_storage);
        // Send next acknowledge if there is something to acknowledge:
        if let Option::Some(seq_num) = next_ack {
            let segment = Segment::Acknowledge { seq_num };
            match stream.listener._send_to(stream.peer_addr, segment) {
                Result::Ok(_) => {},
                Result::Err(error) => {
                    stream.error(error, true);
                    return;
                }
            }
        }
        // Send as many sequential segments as possible if they are avaliable
        loop {
            let mut send_storage = stream.send_storage.write().unwrap();
            let segment = send_storage.pop();
            drop(send_storage);
            if let Option::Some((seq_num, variant)) = segment {
                let segment = Segment::Sequential { seq_num, variant };
                let send_result = stream.listener._send_to(stream.peer_addr, segment);
                if let Result::Err(error) = send_result {
                    stream.error(error, true);
                    return;
                }
            } else {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}