use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    process::exit,
    str::FromStr,
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

// TODO: list alarms, allow removal of alarms (id by hash of alarm)
#[derive(Subcommand)]
enum CliCommand {
    #[command(about = "Print current settings")]
    Info,
    #[command(about = "Print a specific setting")]
    Get { attribute: String },
    #[command(about = "Change a setting")]
    Set { attribute: String, value: String },
    #[command(about = "Kill pwalarmd")]
    Kill,
    #[command(about = "List current alarms")]
    List,
    #[command(about = "Delete alarm")]
    Remove { hash: String },
    #[command(about = "Create new alarm")]
    Add {
        #[clap(short = 'T', long)]
        title: Option<String>,
        #[clap(short, long)]
        desc: Option<String>,
        #[clap(short = 't', long)]
        time: String,
        #[clap(short, long)]
        repeat: Option<String>,
        #[clap(short, long)]
        sound: Option<String>,
        #[clap(short, long)]
        icon: Option<String>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let res = Cli::parse();
    let sock = res
        .socket
        .unwrap_or(format!("/run/user/{}/pwalarmd/pwalarmd.sock", unsafe {
            libc::getuid()
        }));
    match res.cmd {
        CliCommand::Info => {
            let mut socket = UnixStream::connect(&sock)?;
            let mut r: RequestSuccessWithData;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::Sound)?;
            r = recv_get(&mut socket)?;
            println!("sound = {}", if r.has_st() { r.st() } else { "unknown" });
            socket = UnixStream::connect(&sock)?;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::Poll)?;
            r = recv_get(&mut socket)?;
            println!("poll = {}", if r.has_ui() { r.ui() } else { 0 });
            socket = UnixStream::connect(&sock)?;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::Notify)?;
            r = recv_get(&mut socket)?;
            println!("notify = {}", if r.has_bl() { r.bl() } else { false });
            socket = UnixStream::connect(&sock)?;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::AppName)?;
            r = recv_get(&mut socket)?;
            println!("appname = {}", if r.has_st() { r.st() } else { "unknown" });
            socket = UnixStream::connect(&sock)?;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::Daemon)?;
            r = recv_get(&mut socket)?;
            println!("daemon = {}", if r.has_bl() { r.bl() } else { false });
            socket = UnixStream::connect(&sock)?;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::Tpfc)?;
            r = recv_get(&mut socket)?;
            println!("tpfc = {}", if r.has_sui() { r.sui() } else { 0 });
            socket = UnixStream::connect(&sock)?;
            send_get(&mut socket, protobuf_sock::GeneralInfoType::Tsfc)?;
            r = recv_get(&mut socket)?;
            println!("tsfc = {}", if r.has_sui() { r.sui() } else { 0 });
        }
        CliCommand::Get { attribute } => {
            let mut socket = UnixStream::connect(&sock)?;
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
        CliCommand::Set { attribute, value } => {
            let mut socket = UnixStream::connect(&sock)?;
            let mut req = protobuf_sock::SocketRequest::new();
            match attribute.as_str() {
                "sound" => {
                    let mut z = protobuf_sock::ChangeGeneralSound::new();
                    z.set_newsound(value);
                    req.set_cgs(z);
                }
                "poll" => {
                    let mut z = protobuf_sock::ChangePollFrequency::new();
                    let vals: Vec<&str> = value.split(",").collect();
                    if !(vals.len() == 1 || vals.len() == 3) {
                        beprint("invalid number of arguments");
                        beprint("pass either `poll` or 'poll,tpfc,tsfc'");
                        std::process::exit(125);
                    }
                    z.set_poll(vals[0].parse()?);
                    if vals.len() == 3 {
                        z.set_tpfc(vals[1].parse()?);
                        z.set_tsfc(vals[2].parse()?);
                    }
                    req.set_cpf(z);
                }
                "notify" => {
                    let mut z = protobuf_sock::SetNotify::new();
                    match value.as_str() {
                        "true" => z.set_noti(true),
                        "false" => z.set_noti(false),
                        _ => {
                            beprint("not acceptable boolean value");
                            std::process::exit(124);
                        }
                    }
                    req.set_sn(z);
                }
                "can" => {
                    let mut z = protobuf_sock::ChangeAppName::new();
                    z.set_newname(value);
                    req.set_can(z);
                }
                _ => {
                    beprint(&format!("unknown attribute '{}'", &attribute));
                    exit(1);
                }
            }
            req.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
            socket.flush()?;
            if recv(&mut socket)?.has_err() {
                beprint("server error during value set");
                std::process::exit(123);
            }
        }
        CliCommand::Kill => {
            let mut socket = UnixStream::connect(&sock)?;
            let mut sr = protobuf_sock::SocketRequest::new();
            sr.set_ks(protobuf_sock::KillSwitch::new());
            sr.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
            socket.flush()?;
        }
        CliCommand::List => {
            let mut socket = UnixStream::connect(&sock)?;
            let mut sr = protobuf_sock::SocketRequest::new();
            sr.set_fa(protobuf_sock::FetchAlarms::new());
            sr.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
            socket.flush()?;
            let resp = recv(&mut socket)?;
            if !resp.has_swa() {
                beprint("could not receive alarms");
                exit(124);
            }
            for m in &resp.swa().als {
                let t = m.time();
                println!(
                    "{:8x}: \"{}\" @ {:02}:{:02}:{:02} (rep: {:?})",
                    _hashalarms(m),
                    m.title(),
                    t / 3600,
                    (t / 60) % 60,
                    t % 60,
                    m.repeat
                );
            }
        }
        CliCommand::Remove { hash } => {
            // TODO: dedup with List
            let h = u32::from_str_radix(&hash, 16)?;
            let mut socket = UnixStream::connect(&sock)?;
            let mut sr = protobuf_sock::SocketRequest::new();
            sr.set_fa(protobuf_sock::FetchAlarms::new());
            sr.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
            socket.flush()?;
            let resp = recv(&mut socket)?;
            if !resp.has_swa() {
                beprint("could not recieve original alarm list");
                exit(123);
            }
            for m in &resp.swa().als {
                if _hashalarms(m) == h {
                    socket = UnixStream::connect(&sock)?;
                    let mut sr = protobuf_sock::SocketRequest::new();
                    let mut ins = protobuf_sock::RemoveAlarm::new();
                    ins.al = protobuf::MessageField(Some(Box::new(m.clone())));
                    sr.set_ra(ins);
                    sr.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
                    socket.flush()?;
                    let res = recv(&mut socket)?;
                    if res.has_err() {
                        beprint(&format!("failed to remove alarm: {}", res.err()));
                        exit(121);
                    }
                    exit(0);
                }
            }
            beprint("alarm does not exist");
            exit(122);
        }
        CliCommand::Add {
            title,
            desc,
            time,
            repeat,
            sound,
            icon,
        } => {
            let tc = time
                .splitn(3, ':')
                .map(|z| {
                    u32::from_str(z).unwrap_or_else(|_| {
                        beprint("invalid time specifier");
                        exit(121)
                    })
                })
                .collect::<Vec<_>>();
            let tv: u32 = match tc.len() {
                2 => tc[0] * 3600 + tc[1] * 60,
                3 => tc[0] * 3600 + tc[1] * 60 + tc[2],
                _ => {
                    beprint("invalid time specifier");
                    exit(121);
                }
            };
            let mut v = vec![];
            if let Some(z) = repeat {
                if z.len() != 0 {
                    for m in z.split(",") {
                        v.push(m.to_string());
                    }
                }
            }
            let mut socket = UnixStream::connect(&sock)?;
            let mut sr = protobuf_sock::SocketRequest::new();
            let mut qu = protobuf_sock::NewAlarm::new();
            let mut al = protobuf_sock::AlarmInfo::new();
            al.title = title;
            al.desc = desc;
            al.repeat = v;
            al.sound = sound;
            al.icon = icon;
            al.time = Some(tv);
            qu.al = protobuf::MessageField(Some(Box::new(al)));
            sr.set_na(qu);
            sr.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
            socket.flush()?;
            if recv(&mut socket)?.has_err() {
                beprint("unable to create alarm");
                exit(120);
            }
        }
    }
    Ok(())
}

fn _hashalarms(a: &protobuf_sock::AlarmInfo) -> u32 {
    crc32fast::hash(
        format!(
            "{},{},{},{:?},{},{}",
            a.title(),
            a.desc(),
            a.time(),
            a.repeat,
            a.sound(),
            a.icon()
        )
        .as_bytes(),
    )
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
            }
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
