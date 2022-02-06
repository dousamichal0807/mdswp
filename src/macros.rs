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

macro_rules! conn_already_shut_down {
    () => { std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "Connection already shut down"
    ) };
}

macro_rules! conn_write_finished {
    () => { std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "Sending data to peer has been already finished"
    ) }
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

macro_rules! invalid_ack_num {
    () => { std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "Invalid acknowledgement segment sent by peer"
    ) };
}