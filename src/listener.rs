//! Module for [`MdswpListener`](crate::MdswpListener) and its related data
//! structures.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};

pub struct MdswpListener {
    socket: UdpSocket,
    conn_states: HashMap<SocketAddr, ConnectionState>
}

enum ConnectionState {}