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
mod storage;

#[doc(hidden)]
#[macro_use]
mod util;

use std::io;
use std::io::{stdin, stdout, Write};
use std::net::UdpSocket;

pub use crate::listener::MdswpListener;
pub use crate::stream::MdswpStream;

fn main() -> io::Result<()> {
    let mut buf = String::new();

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    println!("Created a socket local={}", socket.local_addr()?);
    stdout().flush()?;
    stdin().read_line(&mut buf)?;

    socket.connect("192.168.1.1:12345")?;
    println!("Bound socket to {}", socket.peer_addr()?);
    stdout().flush()?;
    stdin().read_line(&mut buf)?;

    let socket2 = UdpSocket::bind("0.0.0.0:0")?;
    println!("Created another socket local={}", socket2.local_addr()?);
    stdout().flush()?;
    stdin().read_line(&mut buf)?;

    Result::Ok(())
}












