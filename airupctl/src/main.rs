use nng::{Socket, Protocol};
use clap::{App, Arg, SubCommand};

fn main() {
    let matches = App::new("Airup Controller")
        .version(env!("CARGO_PKG_VERSION"))
        .about("The controller of airupd init.")
        .subcommand(
        	SubCommand::with_name("sys")
        	    .version(env!("CARGO_PKG_VERSION"))
        	    .about("To control power or system settings.")
        	    .arg(
        	    	Arg::with_name("power")
        	    	    .long("power")
        	    	    .help("OFF or REBOOT.")
        	    	    .value_name("STAT")
        	    	    .takes_value(true)
        	    )
        )
    .get_matches();
    let client = Socket::new(Protocol::Req0).unwrap();
    let addr = "tcp://127.0.0.1:61257";
    match matches.subcommand() {
    	("sys", Some(x)) => {
    		client.dial(addr).unwrap();
    		let action = x.value_of("power").unwrap().to_lowercase();
    		let mut msg = String::new();
    		if action == "off" {
    			msg.push_str("system poweroff");
    		} else if action == "reboot" {
    			msg.push_str("system restart");
    		}
    		let msg = msg.as_bytes();
    		client.send(msg).unwrap();
    	},
    	_ => (),
    };
}
