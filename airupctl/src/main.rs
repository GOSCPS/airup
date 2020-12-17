use nng::{Socket, Protocol};

fn main() {
    let server = Socket::new(Protocol::Req0).unwrap();
    server.dial("tcp://127.0.0.1:61257").unwrap();
    server.send("system poweroff".as_bytes()).unwrap();
    println!("{}", String::from_utf8_lossy(&server.recv().unwrap()));
}
