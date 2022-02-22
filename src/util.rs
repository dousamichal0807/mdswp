use lazy_static::lazy_static;

use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;

lazy_static! {
    /// Represents a socket address with unspecified IPv4 address and unspecified
    /// port.
    pub(crate) static ref SOCKADDR_V4_ANY: SocketAddr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
    pub(crate) static ref SOCKADDR_V6_ANY: SocketAddr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
    pub(crate) static ref CONNECTION_TIMEOUT: Duration = Duration::from_secs(3);
}

/// Macro cloning all given variables into a closure.
///
/// # Syntax
///
/// ```rust
/// clone!(variable1, variable2, variable3,... => closure)
/// ```
///
/// # Usage
///
/// ```rust
/// use std::sync::Arc;
/// use std::thread;
/// // ...
/// let shared = Arc::new(some_value);
/// let thread = thread::spawn( clone!(shared => move || {
///     // `shared` is cloned into the closure, NOT moved
/// }))
/// // This is possible:
/// do_something(shared);
/// ```
macro_rules! clone {
    ( $n0:ident $(, $name:ident)* => $cls:expr ) => {{
        let $n0 = $n0.clone();
        $( let $name = $name.clone(); )*
        $cls
    }};
}

macro_rules! conn_does_not_exist {
    () => { std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "Client not connected"
    ) }
}

macro_rules! conn_invalid_ack_segment {
    () => { std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "Invalid acknowledgement segment sent by peer"
    ) };
}

macro_rules! conn_reset_by_peer {
    () => { std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "Connection has been reset by peer"
    ) }
}

macro_rules! conn_unexpected_segment {
    ($s:expr) => { std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        format!("Peer sent unexpected segment: {}", $s)
    ) }
}

macro_rules! conn_write_finished {
    () => { std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "Sending data to peer has been already finished"
    ) }
}

pub(crate) use clone;
pub(crate) use conn_does_not_exist;
pub(crate) use conn_invalid_ack_segment;
pub(crate) use conn_reset_by_peer;
pub(crate) use conn_unexpected_segment;
pub(crate) use conn_write_finished;