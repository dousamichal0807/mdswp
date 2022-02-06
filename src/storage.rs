use crate::segment::{MAX_DATA_LEN, SequenceNumber};
use crate::segment::Segment;
use crate::segment::SequentialSegment;
use std::collections::VecDeque;
use std::convert::TryInto;
use std::io;
use std::io::Write;
use std::net::Shutdown;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;

pub(crate) const WINDOW_SIZE: u8 = 5;

/// Structure where all data for given stream is stored.
pub(crate) struct StreamDataStorage {
    socket: Arc<UdpSocket>,
    send_storage: RwLock<SendDataStorage>,
    recv_storage: RwLock<RecvDataStorage>,
    error: RwLock<io::Result<()>>,
}

impl StreamDataStorage {
    /// Creates a new [`StreamDataStorage`] instance.
    pub fn new(socket: UdpSocket) -> Self {
        Self {
            socket: Arc::new(socket),
            send_storage: RwLock::new(SendDataStorage::new()),
            recv_storage: RwLock::new(RecvDataStorage::new()),
            error: RwLock::new(Result::Ok(())),
        }
    }

    /// Returns:
    ///
    /// - [`Result::Err`] if stream is closed
    /// - [`Result::Ok`] if stream is open
    pub fn assert_alive(&self) -> io::Result<()> {
        let result = self.error.read().unwrap();
        match &*result {
            Result::Ok(()) => Result::Ok(()),
            Result::Err(err) => Result::Err(io::Error::new(err.kind(), err.to_string()))
        }
    }

    /// Returns reference to the underlying UDP socket.
    pub fn socket(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }

    /// Returns if the write has been completed.
    pub fn write_finished(&self) -> bool {
        self.send_storage.read().unwrap().has_finished()
    }

    /// Returns if the stream was shut down because of an error.
    pub fn is_err(&self) -> bool {
        self.error.read().unwrap().is_err()
    }

    pub fn register_received_data(&self, data: &[u8]) {
        match data.try_into() {
            Result::Err(err) => self.shutdown_with_error(err),
            Result::Ok(Segment::Reset) => self.shutdown_with_error(conn_reset_by_peer!()),
            Result::Ok(Segment::Acknowledge { seq_num }) => {
                let mut send_storage = self.send_storage.write().unwrap();
                match send_storage.register_acknowledge(seq_num) {
                    Result::Err(err) => self.shutdown_with_error(err),
                    Result::Ok(..) => {}
                }
            }
            Result::Ok(Segment::Sequential { seq_num, variant }) => todo!(),
            Result::Ok(other) => self.shutdown_with_error(conn_unexpected_segment!(other))
        };
    }

    pub fn register_acknowledge(&self, seq_num: SequenceNumber) {
        match self.send_storage.write().unwrap().register_acknowledge(seq_num) {
            Result::Ok(()) => {},
            Result::Err(err) => self.shutdown_with_error(err)
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        todo!()
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        self.send_storage.write().unwrap().write(buf)
    }

    pub fn flush(&self) -> io::Result<()> {
        self.send_storage.write().unwrap().flush()
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match how {
            Shutdown::Read => todo!(),
            Shutdown::Write => self.send_storage.write().unwrap().finish(),
            Shutdown::Both => todo!()
        }
    }

    pub fn shutdown_with_error(&self, err: io::Error) {
        *self.error.write().unwrap() = Result::Err(err);
    }
}

/// Structure where all sent data is stored.
struct SendDataStorage {
    buffer: Vec<u8>,
    window: (u8, u8),
    data: VecDeque<(SequentialSegment, Option<Instant>)>,
}

impl SendDataStorage {
    /// Creates a new instance of [`SendDataStorage`].
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            window: (0, WINDOW_SIZE),
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
    pub fn register_acknowledge(&mut self, seq_num: u8) -> io::Result<()> {
        let (win_start, win_end) = self.window;
        // Index in the queue
        let (index, _) = seq_num.overflowing_sub(win_start);
        let index_usize = index as usize;
        // If index is out of range
        if index >= WINDOW_SIZE || index_usize >= self.data.len() {
            return Result::Err(invalid_ack_num!());
        }
        // Slide the window:
        let new_win_start = win_start.overflowing_add(index).0;
        let new_win_end = win_end.overflowing_add(index).0;
        self.window = (new_win_start, new_win_end);
        // Drain data that will not be sent again
        self.data.drain(0..=index_usize);
        // Return
        Result::Ok(())
    }

    pub fn update_send_time(&mut self, seq_num: u8) -> io::Result<()> {
        let (win_start, win_end) = self.window;
        // Index in the queue
        let (index, _) = seq_num.overflowing_sub(win_start);
        let index_usize = index as usize;
        // If index is out of range
        if index >= WINDOW_SIZE || index_usize >= self.data.len() {
            return Result::Err(invalid_ack_num!());
        }
        // Update send time:
        self.data[index_usize].1 = Option::Some(Instant::now());
        Result::Ok(())
    }

    /// Returns if the sending data has ended.
    pub fn has_finished(&self) -> bool {
        if self.data.is_empty() { return false; }
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

struct RecvDataStorage {
    buffer: VecDeque<u8>,
    window: (u8, u8)
}

impl RecvDataStorage {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
            window: (0, WINDOW_SIZE)
        }
    }
}