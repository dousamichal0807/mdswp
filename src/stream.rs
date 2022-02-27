//! Module for [`MdswpStream`](crate::MdswpStream) and its related data structures.

#[doc(hidden)] pub(crate) mod inner;
#[doc(hidden)] mod recv;
#[doc(hidden)] mod send;

use std::io;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::sync::Arc;

use self::inner::StreamInner;

/// A struct for communicating through the stream. This struct implements [`Read`] and [`Write`]
/// like the standard [`TcpStream`].
///
/// [`Read`]: std::io::Read
/// [`Write`]: std::io::Write
/// [`TcpStream`]: std::net::TcpStream
pub struct MdswpStream(
    #[doc(hidden)]
    pub(crate) Arc<StreamInner>
);

impl MdswpStream {
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
    pub fn connect<A>(addr: A) -> io::Result<Self>
        where A: ToSocketAddrs {
        Result::Ok(Self(StreamInner::connect(addr)?))
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
    pub fn connect_to(addr: SocketAddr) -> io::Result<Self> {
        Result::Ok(Self(StreamInner::connect_to(addr)?))
    }

    /// Returns the socket address of the remote peer of this MDSWP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.0.peer_addr()
    }

    /// Returns the socket address of the local half of this MDSWP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }

    /// Shuts down the connection by sending a [`Reset`](Segment::Reset) segment to
    /// the peer.
    ///
    /// Calling this function multiple times will result in an error.
    pub fn reset(&mut self) -> io::Result<()> {
        self.0.reset()
    }

    /// Returns:
    ///
    ///  -  [`Result::Ok`] if stream has not errored
    ///  -  [`Result::Err`] with appropriate message, if stream has errored
    pub fn stream_state(&self) -> io::Result<()> {
        self.0.stream_state()
    }

    /// Returns if the stream has errored.
    pub fn is_err(&self) -> bool {
        self.0.is_err()
    }

    /// Finishes the write operation to the stream.
    ///
    /// Calling [`write`](Self::write) method after [`finish_write`] will result in
    /// an error. Same rule applies for calling [`finish_write`] more than once.
    pub fn finish_write(&mut self) -> io::Result<()> {
        self.0.finish_write()
    }

    /// Returns if writing has been completed by the [`finish_write`] method.
    pub fn is_write_finished(&self) -> io::Result<bool> {
        self.0.is_write_finished()
    }

    /// Returns if peer has sent its all data. If this method returns `true`, it
    /// indicates that there will be no more data and calls to [`read`](Self::read)
    /// will result in an error.
    pub fn is_read_finished(&self) -> io::Result<bool> {
        self.0.is_read_finished()
    }

    /// Sets the TTL for the underlying UDP socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.0.set_ttl(ttl)
    }

    /// Returns the TTL of the underlying UDP socket.
    pub fn ttl(&self) -> io::Result<u32> {
        self.0.ttl()
    }

    /// Creates a new independently owned handle to the underlying socket.
    ///
    /// The returned [`MdswpStream`] is a reference to the same stream that this object references.
    /// Both handles will read and write the same stream of data, and options set on one stream will
    /// be propagated to the other stream.
    pub fn try_clone(&self) -> io::Result<Self> {
        self.stream_state()
            .and_then(|_| Result::Ok(MdswpStream(self.0.clone())))
    }
}

impl Read for MdswpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for MdswpStream {
    /// Writes some bytes to the stream. Note that this stream is buffered and data
    /// is not sent immediately. It is always guaranteed that all data is written to
    /// the stream when calling this method.
    ///
    /// # Return value
    ///
    ///  -  [`Result::Ok`] if there was no error.
    ///  -  [`Result::Err`] if either:
    ///      -  connection was reset
    ///      -  connection has errored (see [`is_err`] method)
    ///      -  [`finish_write`] method was called
    ///
    /// [`finish_write`]: MdswpStream::finish_write
    /// [`is_err`]: MdswpStream::is_err
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    /// Flushes the stream, e.g. forces sending all remaining bytes even if there is
    /// small amount of data to be sent.
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Drop for MdswpStream {
    fn drop(&mut self) {
        drop(())
    }
}