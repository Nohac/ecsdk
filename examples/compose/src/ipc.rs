use std::io;

use interprocess::local_socket::{
    GenericFilePath, ListenerOptions,
    tokio::{Listener, Stream, prelude::*},
};

pub const SOCKET_PATH: &str = "/tmp/ecs-compose-daemon.sock";

pub fn socket_name() -> interprocess::local_socket::Name<'static> {
    SOCKET_PATH.to_fs_name::<GenericFilePath>().unwrap()
}

pub fn create_listener() -> io::Result<Listener> {
    let _ = std::fs::remove_file(SOCKET_PATH);
    ListenerOptions::new().name(socket_name()).create_tokio()
}

pub async fn connect() -> io::Result<Stream> {
    Stream::connect(socket_name()).await
}
