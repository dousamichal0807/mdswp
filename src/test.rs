use std::io::{Read, Write};
use std::{io, thread};
use std::fs::File;
use std::thread::JoinHandle;
use crate::MdswpListener;
use crate::MdswpStream;
use crate::segment::MAX_DATA_LEN;

static TEST_FILE: &[u8] = include_bytes!("/usr/bin/bash");

#[test]
fn file_send() {
    let recv_thread: JoinHandle<io::Result<()>> = thread::spawn(|| {
        let mut listener = MdswpListener::bind("[::1]:12345")?;
        let (mut stream, _) = listener.accept()?;
        let mut contents = Vec::new();
        let mut buf = [0; MAX_DATA_LEN];
        while !stream.is_read_finished()? {
            let len = stream.read(&mut buf)?;
            contents.extend_from_slice(&buf[..len]);
            println!("Received {} bytes", len);
        }
        println!("All data read");
        // Write to file (in case of failed test we can look what's wrong):
        let mut file = File::create("/tmp/received")?;
        file.write_all(&contents)?;
        // Do assertion
        assert!(contents == TEST_FILE, "Send and received data do not match!");
        Result::Ok(())
    });
    let send_thread: JoinHandle<io::Result<()>> = thread::spawn(|| {
        let mut stream = MdswpStream::connect("[::1]:12345")?;
        stream.write(TEST_FILE)?;
        stream.flush()?;
        stream.finish_write()?;
        Result::Ok(())
    });
    println!("send: {:?}", send_thread.join().unwrap());
    println!("recv: {:?}", recv_thread.join().unwrap());
}