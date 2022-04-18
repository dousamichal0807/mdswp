//! Module for [`MdswpListener`](crate::MdswpListener) and its related data
//! structures.

#[doc(hidden)] pub(crate) mod inner;

use std::io;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::sync::Arc;

use crate::MdswpStream;

use self::inner::ListenerInner;

/// A MDSWP socket, listening for connections.
///
/// After creating a [`MdswpStream`] by binding it to a socket address, it listens
/// for incoming TCP connections. These can be accepted by calling
/// [`accept`](Self::accept) or by iterating over the [`Incoming`] iterator returned
/// by [`incoming`](Self::incoming) method.
///
/// The socket will be closed when the value is dropped.
pub struct MdswpListener (pub(crate) Arc<ListenerInner>);

impl MdswpListener {
    /// Creates a new [`MdswpListener`] which will be bound to the specified
    /// address.
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port to
    /// this listener. The port allocated can be queried via the
    /// [`local_addr`](Self::local_addr) method.
    ///
    /// The address type can be any implementor of [`ToSocketAddrs`] trait. See its
    /// documentation for concrete examples.
    ///
    /// If `addr` argument yields multiple addresses, bind will be attempted with
    /// each of the addresses until one succeeds and returns the listener. If none
    /// of the addresses succeed in creating a listener, the error returned from the
    /// last attempt (the last address) is returned.
    ///
    /// # Examples
    ///
    /// Creates a MDSWP listener bound to `127.0.0.1:80`:
    ///
    /// ```rust
    /// use mdswp::MdswpListener;
    ///
    /// let listener = MdswpListener::bind("127.0.0.1:80").unwrap();
    /// ```
    ///
    /// Creates a MDSWP listener bound to `127.0.0.1:80`. If that fails, create
    /// a MDSWP listener bound to `127.0.0.1:443`:
    ///
    /// ```rust
    /// use std::net::SocketAddr;
    /// use mdswp::MdswpListener;
    ///
    /// let addrs = [
    ///     SocketAddr::from(([127, 0, 0, 1], 80)),
    ///     SocketAddr::from(([127, 0, 0, 1], 443)),
    /// ];
    /// let listener = MdswpListener::bind(&addrs[..]).unwrap();
    /// ```
    pub fn bind<A>(addr: A) -> io::Result<Self>
    where A: ToSocketAddrs {
        Result::Ok(Self(ListenerInner::bind(addr)?))
    }

    /// Creates an [`Incoming`] struct from the [`MdswpListener`]. Usage of this method should be
    /// done in a `for` loop; see example below.
    ///
    /// # Example
    ///
    /// ```rust
    /// use mdswp::MdswpListener;
    /// use std::io;
    ///
    /// fn main() -> io::Result<()> {
    ///     let listener = MdswpListener::bind(some_address)?;
    ///     // Infinitely listen for incoming connections:
    ///     for connection in listener.incoming() {
    ///         // do something useful here...
    ///     }
    ///     // Return value:
    ///     Result::Ok(())
    /// }
    /// ```
    pub fn incoming(self) -> Incoming {
        self.into()
    }

    /// Returns if the listener has been shut down by an unexpected error.
    pub fn is_err(&self) -> bool {
        self.0.is_err()
    }

    /// Returns the listener state:
    ///
    ///  -  [`Result::Err`] if error has occurred
    ///  -  [`Result::Ok`] otherwise
    pub fn listener_state(&self) -> io::Result<()> {
        self.0.listener_state()
    }

    /// Returns local address of the underlying UDP socket.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }

    /// Accept a new incoming connection from this listener in non-blocking manner.
    /// For blocking variant see [`accept`](Self::accept) method.
    ///
    /// When established, the corresponding [`MdswpStream`] and the remote peer’s
    /// address will be returned.
    pub fn accept_nonblocking(&mut self) -> io::Result<Option<(MdswpStream, SocketAddr)>> {
        self.0.accept_nonblocking()
    }

    /// Accept a new incoming connection from this listener.
    ///
    /// This function will block the calling thread until a new MDSWP connection is
    /// established. For non-blocking variant see
    /// [`accept_nonblocking`](Self::accept_nonblocking).
    ///
    /// When established, the corresponding [`MdswpStream`] and the remote peer’s
    /// address will be returned.
    pub fn accept(&mut self) -> io::Result<(MdswpStream, SocketAddr)> {
        self.0.accept()
    }

    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl(&mut self, ttl: u32) -> io::Result<()> {
        self.0.set_ttl(ttl)
    }

    /// Returns set time-to-live value.
    ///
    /// See [`set_ttl`](Self::set_ttl) for more details.
    pub fn ttl(&self) -> io::Result<u32> {
        self.0.ttl()
    }

    /// Clones the handle of the [`MdswpListener`]. If [`MdswpListener`] has errored,
    /// this method also returns [`Result::Err`].
    pub fn try_clone(&self) -> io::Result<Self> {
        self.listener_state()?;
        Result::Ok(Self(self.0.clone()))
    }
}

/// An iterator that infinitely accepts connections on a [`MdswpListener`].
///
/// This struct is created by the [`MdswpListener::incoming`] method. See its
/// documentation for more.
pub struct Incoming {
    listener: MdswpListener
}

impl From<MdswpListener> for Incoming {
    /// Creates [`Incoming`] instance from given [`MdswpListener`]
    fn from(listener: MdswpListener) -> Self {
        Self { listener }
    }
}

impl Into<MdswpListener> for Incoming {
    /// Unwraps inner [`MdswpListener`] from given [`Incoming`] struct.
    fn into(self) -> MdswpListener {
        self.listener
    }
}

impl Iterator for Incoming {
    type Item = io::Result<(MdswpStream, SocketAddr)>;

    /// Returns the next connection that was established by peer. This implementation blocks thread
    /// execution until some connection is established.
    ///
    /// # Return value
    ///
    /// [`Option::Some`] containing a new connection.
    fn next(&mut self) -> Option<Self::Item> {
        Option::Some(self.listener.accept())
    }
}