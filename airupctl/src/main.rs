use nng::*;
use ansi_term::Colour::*;
use clap::{App, Arg, SubCommand};

fn main() {
    let client = Socket::new(Protocol::Req0).unwrap();
    let rslt = client.dial("tcp://localhost:11257");
    let matches = App::new("airupctl")
        .version("9999")
        .about("The controller of The Airup Init Scheme")
        .subcommand(SubCommand::with_name("svc")
                    .about("Controller of registried services.")
                    .arg(Arg::with_name("stop")
                         .long("stop")
                         .value_name("SVCNAME")
                         .help("To stop a running service.")
                         .takes_value(true)
                         .conflicts_with_all(&["start", "restart"])
                         )
                    .arg(Arg::with_name("start")
                         .long("start")
                         .value_name("SVCNAME")
                         .help("To start a registried service.")
                         .takes_value(true)
                         .conflicts_with_all(&["stop", "restart"])
                    )
                    .arg(Arg::with_name("restart")
                         .long("restart")
                         .value_name("SVCNAME")
                         .help("To restart a running service.")
                         .takes_value(true)
                         .conflicts_with_all(&["start", "stop"])
                        )
                )
        .get_matches();
    match matches.subcommand_name() {
        Some("svc") => {
            let matches = matches.subcommand_matches("svc").unwrap();
            rslt.unwrap();
            let mut cmd = String::from("service ");
            let p:&str;
            if matches.is_present("start") {
                cmd.push_str("start ");
                p = "start";
            } else if matches.is_present("stop") {
                cmd.push_str("stop ");
                p = "stop";
            } else if matches.is_present("restart") {
                cmd.push_str("restart");
                p = "restart";
            } else {
                eprintln!("{}'svc' needs an argument.", Red.paint(" * "));
                return;
            }
            cmd.push_str(matches.value_of(p).unwrap());
            client.send(cmd.as_bytes()).unwrap();
        },
        _ => (),
    }
}
