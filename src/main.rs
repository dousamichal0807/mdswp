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

#[doc(hidden)]
mod segment;

#[doc(hidden)]
#[macro_use]
mod util;

use std::io;
use std::net::TcpListener;
use std::net::TcpStream;
use std::thread;

pub use crate::listener::MdswpListener;
pub use crate::stream::MdswpStream;

fn main() -> thread::Result<()> {
    let listener_thread = thread::spawn(listener_thread);
    let client_thread = thread::spawn(client_thread);
    let _ = listener_thread.join()?;
    let _ = client_thread.join()?;
    Result::Ok(())
}

fn listener_thread() -> io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:12345")?;
    println!("listener local: {:?}", listener.local_addr()?);
    let (stream, _) = listener.accept()?;
    println!("server local: {:?}", stream.local_addr()?);
    println!("server remote: {:?}", stream.peer_addr()?);
    Result::Ok(())
}

fn client_thread() -> io::Result<()> {
    let stream = TcpStream::connect("127.0.0.1:12345")?;
    println!("client local: {:?}", stream.local_addr()?);
    println!("client remote: {:?}", stream.peer_addr()?);
    Result::Ok(())
}