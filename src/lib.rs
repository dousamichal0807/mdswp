//! This crate implements a SWP (sliding window protocol). MDSWP is built on top of
//! the UDP (User Datagram Protocol).
//!
//! MDSWP calls its own PDUs *segments*. See [`segment`] to learn how MDSWP works
//! under the hood.
//!
//! # Basic usage
//!
//! Using MDSWP is done by using two structures: [`MdswpListener`] and
//! [`MdswpStream`].
//!
//! [`MdswpListener`] is used to listen on given address for incoming connections.
//! [`MdswpListener::accept`] method is used to accept next connection. This method
//! returns a [`MdswpStream`], which you can send and receive data from. Note that
//! [`MdswpListener`] does *not* establish any connection.
//!
//! For establishing a connection, [`MdswpStream::connect`] method is used. Again,
//! the return value is of type [`MdswpStream`]. Then it is used in the same way as
//! [`MdswpStream`] instance returned by accepting an incoming connection.

pub mod listener;
pub mod stream;

mod segment;

#[doc(hidden)]
#[macro_use]
mod util;

#[doc(hidden)]
#[cfg(test)]
mod test;

pub use crate::listener::MdswpListener;
pub use crate::stream::MdswpStream;