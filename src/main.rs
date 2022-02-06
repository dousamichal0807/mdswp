mod listener;
#[macro_use] mod macros;
mod segment;
mod storage;
mod stream;
mod threading;

use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::net::UdpSocket;
pub use stream::MdswpStream;

fn main() -> std::io::Result<()> {
    let addr: SocketAddr = (Ipv4Addr::UNSPECIFIED, 0).into();
    let recv = UdpSocket::bind(addr)?;
    println!("{}", recv.local_addr().unwrap());
    Result::Ok(())
}