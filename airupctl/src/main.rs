use nng::*;

fn main() {
    let client = Socket::new(Protocol::Req0).unwrap();
    client.dial("tcp://localhost:11257").unwrap();
    client.send("service stop a".as_bytes()).unwrap();
    println!("{:?}", &client.recv().unwrap()[..]);
}
