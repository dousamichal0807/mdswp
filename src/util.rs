use std::fmt::Display;
use std::io;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::time::Duration;

use crate::segment::SeqNumber;

pub(crate) const WINDOW_SIZE: SeqNumber = 5;

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

pub(crate) fn clone_io_err(err: &io::Error) -> io::Error {
    io::Error::new(err.kind(), err.to_string())
}

pub(crate) fn conn_invalid_seq_num(seq_num: SeqNumber) -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("Sent segment with invalid sequence number: {}", seq_num)
    )
}

pub(crate) fn conn_reset_by_peer() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionReset,
        "Connection has been reset by peer"
    )
}

pub(crate) fn conn_reset_local() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionReset,
        "Connection has been reset locally"
    )
}

pub(crate) fn conn_timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "Peer did not respond"
    )
}

pub(crate) fn conn_unexpected_segment<D>(segment: D) -> io::Error
where D: Display {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("Peer sent unexpected segment: {}", segment)
    )
}

pub(crate) fn conn_write_finished() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "Sending data to peer has been already finished"
    )
}

pub(crate) fn sock_addr_v4_any() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
}

pub(crate) fn sock_addr_v6_any() -> SocketAddr {
    SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
}

pub(crate) fn conn_timeout() -> Duration {
    Duration::from_secs(3)
}

pub(crate) use clone;