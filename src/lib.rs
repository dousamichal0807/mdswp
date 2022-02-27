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

pub use crate::listener::MdswpListener;
pub use crate::stream::MdswpStream;












