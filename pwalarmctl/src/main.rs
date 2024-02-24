use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    process::exit,
};

use clap::{Parser, Subcommand};
use colored::Colorize;
use protobuf::Message;
use protobuf_sock::{ErrorReason, RequestSuccessWithData};

mod protobuf_sock;

const BUF_SIZE: usize = 16384;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    socket: Option<String>,
    #[command(subcommand)]
    cmd: CliCommand,
}

// TODO: list alarms, allow removal of alarms
#[derive(Subcommand)]
enum CliCommand {
    #[command(about = "print current settings")]
    Info,
    Get {
        attribute: String,
    },
    Set {
        attribute: String,
    },
    Kill,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let res = Cli::parse();
    let sock = res
        .socket
        .unwrap_or(format!("/run/user/{}/pwalarmd/pwalarmd.sock", unsafe {
            libc::getuid()
        }));
    match res.cmd {
        CliCommand::Get { attribute } => {
            let mut socket = UnixStream::connect(sock)?;
            send_get(
                &mut socket,
                match attribute.as_str() {
                    "sound" => protobuf_sock::GeneralInfoType::Sound,
                    "poll" => protobuf_sock::GeneralInfoType::Poll,
                    "notify" => protobuf_sock::GeneralInfoType::Notify,
                    "appname" => protobuf_sock::GeneralInfoType::AppName,
                    "daemon" => protobuf_sock::GeneralInfoType::Daemon,
                    "tpfc" => protobuf_sock::GeneralInfoType::Tpfc,
                    "tsfc" => protobuf_sock::GeneralInfoType::Tsfc,
                    _ => {
                        beprint(&format!("unknown attribute '{}'", &attribute));
                        exit(1);
                    }
                },
            )?;
            let resp = recv_get(&mut socket)?;
            if resp.has_st() {
                println!("{}", resp.st());
            } else if resp.has_ui() {
                println!("{}", resp.ui());
            } else if resp.has_bl() {
                println!("{}", resp.bl());
            } else if resp.has_sui() {
                println!("{}", resp.sui());
            } else {
                beprint("server failed to set data type");
                exit(126);
            }
        }
        _ => todo!(),
    }
    Ok(())
}

fn beprint(msg: &str) {
    eprint!("{}", "pwalarmctl".yellow().bold());
    eprintln!(": {}", msg.bright_red());
}

fn send_get(
    socket: &mut UnixStream,
    ty: protobuf_sock::GeneralInfoType,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut req = protobuf_sock::SocketRequest::new();
    let mut g = protobuf_sock::FetchGeneralInfo::new();
    g.set_git(ty);
    req.set_fgi(g);
    req.write_to(&mut protobuf::CodedOutputStream::new(socket))?;
    socket.flush()?;
    Ok(())
}

fn recv(
    socket: &mut UnixStream,
) -> Result<protobuf_sock::SocketResponse, Box<dyn std::error::Error>> {
    let mut buf = [0u8; BUF_SIZE];
    let rc = socket.read(&mut buf)?;
    Ok(protobuf_sock::SocketResponse::parse_from_bytes(&buf[..rc])?)
}

fn recv_get(socket: &mut UnixStream) -> Result<RequestSuccessWithData, Box<dyn std::error::Error>> {
    let mut resp = recv(socket)?;
    if resp.has_err() {
        match ri_to_rr(
            resp.take_err()
                .er
                .ok_or("response did not set error")?
                .enum_value(),
        )? {
            ErrorReason::ParseFailureError => {
                beprint("protocol transmission failure");
                exit(2);
            }
            ErrorReason::MissingRequiredComponent => {
                beprint("request was sent malformed");
                exit(3);
            }
            ErrorReason::IllegalEnumOption => {
                beprint("server does not support enum type");
                exit(4);
            }
            _ => {
                beprint("server returned non-standard error");
                exit(5);
            },
        }
    }
    if !resp.has_swd() {
        beprint("server returned unreasonable response");
        exit(127);
    }
    Ok(resp.take_swd())
}

fn ri_to_rr<T>(i: Result<T, i32>) -> Result<T, &'static str> {
    if let Ok(z) = i {
        Ok(z)
    } else {
        Err("failed enum conversion")
    }
}
