//! This crate implements a SWP (sliding window protocol). This protocol is built
//! on top of UDP. This crate calls its own PDUs *segments*.
//!
//! # Segments
//!
//! ```text
//!                            +----------------+
//!                            |  ALL SEGMENTS  |
//!                            +----------------+
//!                                     |
//!                 +-------------------+-------------------+
//!                 |                   |                   |
//!        +-----------------+ +-----------------+ +-----------------+
//!        |   Sequential    | |   Acknowledge   | | Non-sequential  |
//!        +-----------------+ +-----------------+ +-----------------+
//!                 |                                       |
//!      +----------+----------+                     +------+------+
//!      |          |          |                     |             |
//! +--------+ +--------+ +--------+           +-----------+ +-----------+
//! | Accept | | Finish | |  Data  |           | Establish | |   Reset   |
//! +--------+ +--------+ +--------+           +-----------+ +-----------+
//! ```
//!
//! Segment can be of several types. Segments are divided into two types: sequential
//! and non-sequential. Successful transmission of sequential segment must be
//! confirmed by the `Acknowledge` segment, whereas non-sequential are not
//! *acknowledged*. `Acknowledge` segment is not acknowledged again since that would
//! result in a *acknowledge storm*, but in the diagram above it is separated from
//! non-sequential segments for its special meaning.
//!
//! To distinguish between segment types in the UDP datagram, each segment type is
//! given a unique instruction code. Instruction code occupies the first byte in the
//! segment. Put another way, segment type is determined by the first byte of the
//! segment, where the instruction code is located.
//!
//! # Opening a connection
//!
//! TODO: 3-way handshake
//!
//! # Closing a connection
//!
//! TODO: FINISH segment, RESET segment
//!
//! # Sending data
//!
//! TODO: DATA segment

pub mod listener;
pub mod stream;

mod segment;

#[doc(hidden)]
#[macro_use]
mod util;

use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Write;
use std::thread;

pub use crate::listener::MdswpListener;
pub use crate::stream::MdswpStream;

fn main() -> io::Result<()> {
    let recv = thread::spawn(recv_thread);
    let send = thread::spawn(send_thread);
    recv.join().unwrap()?;
    send.join().unwrap()
}

fn recv_thread() -> io::Result<()> {
    let listener = MdswpListener::bind("0.0.0.0:12345")?;
    let (mut stream, _) = listener.accept()?;
    println!("connection accepted");
    //stream.finish_write()?;
    let mut received = Vec::new();
    let mut buf = [0; 512];
    while !stream.is_read_finished()? {
        println!("Try read");
        let len = stream.read(&mut buf)?;
        received.extend_from_slice(&buf[..len]);
    }
    let mut expected = Vec::new();
    let mut file = File::open("/usr/bin/bash")?;
    file.read_to_end(&mut expected)?;
    println!("Are they same? {}", expected == received);
    let mut recvd_write = File::create("/tmp/recv")?;
    recvd_write.write_all(&received)?;
    Result::Ok(())
}

fn send_thread() -> io::Result<()> {
    let mut buf = Vec::new();
    let mut file = File::open("/usr/bin/bash")?;
    file.read_to_end(&mut buf)?;
    let mut stream = MdswpStream::connect("127.0.0.1:12345")?;
    println!("Writing data");
    stream.write_all(&buf)?;
    println!("All data written");
    println!("Finishing");
    stream.finish_write()?;
    println!("MAIN: send thread ends");
    Result::Ok(())
}












