use crate::segment::Segment;
use crate::storage::StreamDataStorage;
use std::convert::TryInto;
use std::io;
use std::sync::Arc;

pub(crate) fn send_thread(data_storage: Arc<StreamDataStorage>) {
    todo!()
}

pub(crate) fn recv_thread(data_storage: Arc<StreamDataStorage>) {
    let socket = data_storage.socket();
    let mut buf = [0; u16::MAX as usize];

    while !data_storage.is_err() {
        let len = match socket.recv(&mut buf) {
            Result::Ok(len) => len,
            Result::Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Result::Err(err) => {
                data_storage.shutdown_with_error(err);
                return;
            }
        };

        match buf[0..len].try_into() {
            Result::Ok(segment) => match segment {
                Segment::Reset => data_storage.shutdown_with_error(conn_reset_by_peer!()),
                Segment::Acknowledge { seq_num } => data_storage.register_acknowledge(seq_num),
                Segment::Sequential { seq_num, variant } => todo!(),
                other => data_storage.shutdown_with_error(conn_unexpected_segment!(other))
            },
            Result::Err(err) => data_storage.shutdown_with_error(conn_unexpected_segment!(err))
        };


    }
}