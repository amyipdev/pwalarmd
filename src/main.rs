// TODO: better error handling
// The only thing that should actually crash is config issues
// (or extraordinary circumstances, like unrepresentable times)
// Otherwise, fall back to a sane default + log
// TODO: pwalarmctl
// send requests (new, modify, remove); save to write to .toml
//   (if and only if user has write access)
use std::{
    cmp::{max, Ordering},
    collections::VecDeque,
    fs::File,
    io::{BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::Path,
    time::Duration,
};

use chrono::{Datelike, Local, NaiveDate, NaiveTime, Weekday};
use colored::Colorize;
use daemonize::Daemonize;
use notify_rust::Notification;
use protobuf::Message;
use rodio::{source::SamplesConverter, Decoder, OutputStream, Source};
use serde_derive::{Deserialize, Serialize};
use toml::value::Datetime;

mod protobuf_sock;
use protobuf_sock::{socket_request, AlarmInfo, ErrorReason, GeneralInfoType};

const BUFFER_READ: usize = 16384;

#[derive(Serialize, Deserialize)]
struct Config {
    #[serde(rename = "General")]
    general: GeneralConfig,
    #[serde(rename = "Alarm")]
    alarms: Option<Vec<Alarm>>,
}

#[derive(Serialize, Deserialize)]
struct GeneralConfig {
    sound: Option<String>,
    poll: Option<u64>,
    notify: bool,
    custom_app_name: Option<String>,
    daemon: Option<bool>,
    tpfc: Option<u16>,
    tsfc: Option<u16>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone)]
struct Alarm {
    title: Option<String>,
    description: Option<String>,
    time: Datetime,
    // ["Mo", "We", ...]
    repeat: Option<Vec<String>>,
    // TODO: allow Volume control, sets system volume (avoids mute)
    sound: Option<String>,
    icon: Option<String>,
}

#[derive(PartialEq, Eq)]
struct LocalAlarm {
    next_run_date: NaiveDate,
    alarm: Alarm,
}
impl PartialOrd for LocalAlarm {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.next_run_date > other.next_run_date {
            Some(Ordering::Greater)
        } else if self.next_run_date < other.next_run_date {
            Some(Ordering::Less)
        } else {
            let t1 = match self.alarm.time.time {
                Some(v) => v,
                None => return None,
            };
            let t2 = match other.alarm.time.time {
                Some(v) => v,
                None => return None,
            };
            Some(NaiveTime::cmp(
                match &NaiveTime::from_hms_opt(t1.hour.into(), t1.minute.into(), t1.second.into()) {
                    Some(v) => v,
                    None => return None,
                },
                match &NaiveTime::from_hms_opt(t2.hour.into(), t2.minute.into(), t2.second.into()) {
                    Some(v) => v,
                    None => return None,
                },
            ))
        }
    }
}
impl Ord for LocalAlarm {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

// TODO: store sounds in /usr/share/pwalarms/*
// Packaging config:
// One in /etc/pwalarmd.toml
// One in /etc/xdg/pwalarmd/pwalarmd.sample.toml
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let uid = unsafe { libc::getuid() };
    let shellex = shellexpand::tilde("~/config/pwalarmd/pwalarmd.toml").to_string();
    let config_path: String = if let Ok(v) = std::env::var("PWALARMD_CONFIG") {
        v.to_string()
    } else if Path::new(&shellex).exists() {
        shellex
    } else if Path::new("/etc/pwalarmd.toml").exists() {
        "/etc/pwalarmd.toml".to_string()
    } else {
        beprint("could not find config file");
        beprint("try copying /etc/xdg/pwalarmd/pwalarmd.sample.toml");
        beprint("to ~/.config/pwalarmd/pwalarmd.toml");
        std::process::exit(1)
    };
    let get_toml = || match toml::from_str(match std::fs::read_to_string(&config_path) {
        Ok(ref s) => s,
        Err(_) => {
            beprint("unable to read config file, aborting");
            std::process::exit(2)
        }
    }) {
        Ok(c) => c,
        Err(e) => {
            beprint("invalid TOML in config file, aborting");
            beprint("TOML error shown below:");
            eprintln!("{}", e);
            std::process::exit(3)
        }
    };
    let mut config: Config = get_toml();
    let tmp_stderr = File::create(format!("/tmp/pwalarmd-{}.err", uid))?;
    // TODO: kill any other pwalarmds running under the same user
    let nd = std::env::var("PWALARMD_NODAEMON");
    let dmzd = nd == Ok("1".to_string())
        || (nd != Ok("0".to_string()) && config.general.daemon != Some(false));
    if dmzd {
        // TODO: more daemon settings
        let mut cd = std::env::current_exe()?;
        cd.pop();
        Daemonize::new()
            .stderr(tmp_stderr)
            .working_directory(cd.clone())
            .start()?;
    }

    // TODO: better error handling
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let mut global_sound = config
        .general
        .sound
        .clone()
        .unwrap_or("assets/hyper-alarm.mp3".to_string());

    // Model for alarms: VecDeque
    // Instead of checking that we're at the exact time for an alarm,
    // We see if we're past it, then push it backwards
    // Do check that the cached date is correct though - handle system sleep,
    // don't want to throw tons of alarms

    // Set up initial alarm list
    let mut alarm_ring: VecDeque<LocalAlarm> = VecDeque::new();
    let mkring =
        |conf: &Config, ar: &mut VecDeque<LocalAlarm>| -> Result<(), Box<dyn std::error::Error>> {
            if let Some(ref alarms) = conf.alarms {
                for alarm in alarms {
                    let atime = alarm.time.time.unwrap();
                    let la = LocalAlarm {
                        next_run_date: match determine_entry_day(atime, &alarm.repeat) {
                            Some(v) => v,
                            None => continue,
                        },
                        alarm: alarm.clone(),
                    };
                    ar.insert(ar.binary_search(&la).unwrap_or_else(|e| e), la);
                }
            }
            Ok(())
        };
    mkring(&config, &mut alarm_ring)?;

    let polts = |conf: &Config| {
        if let Some(t) = conf.general.poll {
            t
        } else {
            125
        }
    };
    let mut polltime = polts(&config);
    // Set up mtime check
    let mut mtime = std::fs::metadata(&config_path)?.modified()?;
    let tpfcs = |conf: &Config| {
        if let Some(t) = conf.general.tpfc {
            t
        } else {
            2
        }
    };
    let tsfcs = |conf: &Config| {
        if let Some(t) = conf.general.tsfc {
            t
        } else {
            1
        }
    };
    let mut tpfc = tpfcs(&config);
    let mut cpfc = tpfc;
    let mut tsfc = tsfcs(&config);
    let mut csfc = tsfc;
    let tgt = format!("/run/user/{}/pwalarmd/pwalarmd.sock", uid);
    std::fs::remove_file(&tgt).unwrap_or(());
    std::fs::create_dir_all(format!("/run/user/{}/pwalarmd", uid))?;
    let sock = UnixListener::bind(tgt)?;
    sock.set_nonblocking(true)?;
    let mut qbuf = Box::new([0u8; BUFFER_READ]);

    // Processing loop
    loop {
        std::thread::sleep(Duration::from_millis(polltime));
        // Check for config changes
        let nmt = std::fs::metadata(&config_path)?.modified()?;
        if cpfc == 0 {
            if nmt > mtime {
                mtime = nmt;
                config = get_toml();
                global_sound = config
                    .general
                    .sound
                    .clone()
                    .unwrap_or("assets/hyper-alarm.mp3".to_string());
                alarm_ring = VecDeque::new();
                mkring(&config, &mut alarm_ring)?;
                polltime = polts(&config);
                tpfc = tpfcs(&config);
                tsfc = tsfcs(&config);
            }
            cpfc = tpfc;
        } else {
            cpfc -= 1;
        }
        // Poll the socket (nonblocking)
        if csfc == 0 {
            match sock.accept() {
                Ok((mut socket, _addr)) => 'L1: {
                    socket.set_nonblocking(false)?;
                    let rc = socket.read(&mut *qbuf)?;
                    let res = &qbuf[..rc];
                    let msg;
                    match protobuf_sock::SocketRequest::parse_from_bytes(&res) {
                        Ok(r) => msg = r,
                        Err(_) => {
                            proto_send_error(ErrorReason::ParseFailureError, &mut socket)?;
                            break 'L1;
                        }
                    }
                    if msg.message.is_none() {
                        proto_send_error(ErrorReason::MissingRequiredComponent, &mut socket)?;
                        break 'L1;
                    }
                    match msg.message.unwrap() {
                        socket_request::Message::Cgs(v) => {
                            if v.newsound.is_none() {
                                proto_send_error(
                                    ErrorReason::MissingRequiredComponent,
                                    &mut socket,
                                )?;
                                break 'L1;
                            }
                            let s = v.newsound.unwrap();
                            config.general.sound = Some(s.clone());
                            global_sound = s;
                        }
                        socket_request::Message::Cpf(v) => {
                            if v.poll.is_none() && v.tpfc.is_none() && v.tsfc.is_none() {
                                proto_send_error(
                                    ErrorReason::MissingRequiredComponent,
                                    &mut socket,
                                )?;
                                break 'L1;
                            }
                            if let Some(z) = v.poll {
                                config.general.poll = Some(z);
                                polltime = z;
                            }
                            if let Some(z) = v.tpfc {
                                config.general.tpfc = Some(z as u16);
                                tpfc = z as u16;
                            }
                            if let Some(z) = v.tsfc {
                                config.general.tsfc = Some(z as u16);
                                tsfc = z as u16;
                            }
                        }
                        socket_request::Message::Sn(v) => {
                            if let Some(z) = v.noti {
                                config.general.notify = z;
                            } else {
                                proto_send_error(
                                    ErrorReason::MissingRequiredComponent,
                                    &mut socket,
                                )?;
                                break 'L1;
                            }
                        }
                        socket_request::Message::Can(v) => {
                            config.general.custom_app_name = v.newname;
                        }
                        socket_request::Message::Fgi(v) => {
                            if v.git.is_none() {
                                proto_send_error(
                                    ErrorReason::MissingRequiredComponent,
                                    &mut socket,
                                )?;
                                break 'L1;
                            }
                            if let Ok(e) = v.git.unwrap().enum_value() {
                                match e {
                                    GeneralInfoType::Sound => {
                                        proto_send_data_st(&mut socket, global_sound.clone())?
                                    }
                                    GeneralInfoType::Poll => {
                                        proto_send_data_ui(&mut socket, polltime)?
                                    }
                                    GeneralInfoType::Notify => {
                                        proto_send_data_bl(&mut socket, config.general.notify)?
                                    }
                                    GeneralInfoType::AppName => proto_send_data_st(
                                        &mut socket,
                                        _get_notiname(&config).to_string(),
                                    )?,
                                    GeneralInfoType::Daemon => {
                                        proto_send_data_bl(&mut socket, dmzd)?
                                    }
                                    GeneralInfoType::Tpfc => {
                                        proto_send_data_sui(&mut socket, tpfc as u32)?
                                    }
                                    GeneralInfoType::Tsfc => {
                                        proto_send_data_sui(&mut socket, tsfc as u32)?
                                    }
                                }
                            } else {
                                proto_send_error(ErrorReason::IllegalEnumOption, &mut socket)?;
                            }
                            break 'L1;
                        }
                        socket_request::Message::Fa(_) => {
                            let mut resp = protobuf_sock::SocketResponse::new();
                            let mut dat = protobuf_sock::RequestSuccessWithAlarms::new();
                            for la in &alarm_ring {
                                let a: Result<AlarmInfo, _> = la.alarm.clone().try_into();
                                if a.is_err() {
                                    proto_send_error(
                                        ErrorReason::InternalServerError,
                                        &mut socket,
                                    )?;
                                    break 'L1;
                                }
                                dat.als.push(a.unwrap());
                            }
                            resp.set_swa(dat);
                            resp.write_to(&mut protobuf::CodedOutputStream::new(&mut socket))?;
                            socket.flush()?;
                            socket.set_nonblocking(true)?;
                            break 'L1;
                        }
                        socket_request::Message::Na(v) => 'La2: {
                            if let Some(a) = v.al.into_option() {
                                let b: Result<Alarm, _> = a.try_into();
                                if let Ok(c) = b {
                                    let d = LocalAlarm {
                                        // unwrap here is safe because TryFrom will always generate valid time
                                        next_run_date: match determine_entry_day(c.time.time.unwrap(), &c.repeat) {
                                            Some(v) => v,
                                            // nonrepeating alarms can silent fail
                                            None => break 'La2
                                        },
                                        alarm: c,
                                    };
                                    alarm_ring.insert(
                                        alarm_ring.binary_search(&d).unwrap_or_else(|e| e),
                                        d,
                                    );
                                } else {
                                    proto_send_error(
                                        ErrorReason::MissingRequiredComponent,
                                        &mut socket,
                                    )?;
                                    break 'L1;
                                }
                            } else {
                                proto_send_error(
                                    ErrorReason::MissingRequiredComponent,
                                    &mut socket,
                                )?;
                                break 'L1;
                            }
                        }
                    }
                    proto_send_success(&mut socket)?;
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::WouldBlock {
                        beprint("unknown socket pickup error");
                        std::process::exit(5);
                    }
                }
            }
            csfc = tsfc;
        } else {
            csfc -= 1;
        }
        // Examine alarm
        if alarm_ring.len() == 0 {
            continue;
        }
        let cdt = Local::now();
        let cat = alarm_ring[0].alarm.time.time.unwrap();
        // TODO: set a maximum delta under which alarms can run (10 mins?)
        if alarm_ring[0].next_run_date <= cdt.date_naive()
            && NaiveTime::from_hms_opt(cat.hour.into(), cat.minute.into(), cat.second.into())
                .unwrap()
                <= cdt.time()
        {
            let mut a = alarm_ring.pop_front().unwrap();
            // An expensive operation yes, but it's only run essentially once per 24 hours max
            // TODO: better caching of loaded sounds
            stream_handle.play_raw(if let Some(ref p) = a.alarm.sound {
                loadsnd(p.clone())?
            } else {
                loadsnd(global_sound.clone())?
            })?;
            // TODO: add another condition once icons are added
            if config.general.notify && (a.alarm.title.is_some() || a.alarm.description.is_some()) {
                let mut noti = Notification::new();
                if let Some(ref t) = a.alarm.title {
                    noti.summary(t);
                }
                if let Some(ref t) = a.alarm.description {
                    noti.body(t);
                }
                if let Some(ref t) = a.alarm.icon {
                    noti.icon(t);
                }
                noti.appname(_get_notiname(&config));
                noti.show()?;
            }
            a.next_run_date = a.next_run_date.succ_opt().ok_or("out of dates")?;
            // this *can* be expensive, but unless the user has tons of
            // weird repeat schedules, it should be cheap
            // best-case O(log n), worst case O(n)
            alarm_ring.insert(alarm_ring.binary_search(&a).unwrap_or_else(|e| e), a);
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

fn _get_notiname<'a>(c: &'a Config) -> &'a str {
    if let Some(ref s) = c.general.custom_app_name {
        s
    } else {
        "pwalarmd"
    }
}

fn find_next_rep(mut base: NaiveDate, rep: &Option<Vec<String>>) -> Option<NaiveDate> {
    if let Some(r) = rep {
        for _ in 0..r.len() {
            if r.contains(&weekday_to_str(base.weekday())) {
                return Some(base);
            }
            // This only fails if we run out of representable dates,
            // at which point we have much bigger problems
            base = base.succ_opt().unwrap();
        }
        return None;
    } else {
        return Some(base);
    }
}

fn beprint(msg: &str) {
    eprint!("{}", "pwalarmd".yellow().bold());
    eprintln!(": {}", msg.bright_red());
}

fn weekday_to_str(weekday: Weekday) -> String {
    match weekday {
        Weekday::Mon => "Mo",
        Weekday::Tue => "Tu",
        Weekday::Wed => "We",
        Weekday::Thu => "Th",
        Weekday::Fri => "Fr",
        Weekday::Sat => "Sa",
        Weekday::Sun => "Su",
    }
    .to_string()
}

fn loadsnd(
    path: String,
) -> Result<SamplesConverter<Decoder<BufReader<File>>, f32>, Box<dyn std::error::Error>> {
    Ok(Decoder::new(BufReader::new(File::open(path)?))?.convert_samples::<f32>())
}

// TODO: evaluate whether setting nonblocking on Stream is necessary
// (or just listener)
fn proto_send_error(
    err: ErrorReason,
    sock: &mut UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut resp = protobuf_sock::SocketResponse::new();
    let mut sr = protobuf_sock::RequestError::new();
    sr.set_er(err);
    resp.set_err(sr);
    resp.write_to(&mut protobuf::CodedOutputStream::new(sock))?;
    sock.flush()?;
    sock.set_nonblocking(true)?;
    Ok(())
}

fn proto_send_success(sock: &mut UnixStream) -> Result<(), Box<dyn std::error::Error>> {
    let mut resp = protobuf_sock::SocketResponse::new();
    resp.set_suc(protobuf_sock::RequestSuccess::new());
    resp.write_to(&mut protobuf::CodedOutputStream::new(sock))?;
    sock.flush()?;
    sock.set_nonblocking(true)?;
    Ok(())
}

fn _proto_send_data(
    sock: &mut UnixStream,
    dat: protobuf_sock::RequestSuccessWithData,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut resp = protobuf_sock::SocketResponse::new();
    resp.set_swd(dat);
    resp.write_to(&mut protobuf::CodedOutputStream::new(sock))?;
    sock.flush()?;
    sock.set_nonblocking(true)?;
    Ok(())
}

fn proto_send_data_st(sock: &mut UnixStream, s: String) -> Result<(), Box<dyn std::error::Error>> {
    let mut dat = protobuf_sock::RequestSuccessWithData::new();
    dat.set_st(s);
    _proto_send_data(sock, dat)
}

fn proto_send_data_ui(sock: &mut UnixStream, ui: u64) -> Result<(), Box<dyn std::error::Error>> {
    let mut dat = protobuf_sock::RequestSuccessWithData::new();
    dat.set_ui(ui);
    _proto_send_data(sock, dat)
}

fn proto_send_data_sui(sock: &mut UnixStream, ui: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut dat = protobuf_sock::RequestSuccessWithData::new();
    dat.set_sui(ui);
    _proto_send_data(sock, dat)
}

fn proto_send_data_bl(sock: &mut UnixStream, bl: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut dat = protobuf_sock::RequestSuccessWithData::new();
    dat.set_bl(bl);
    _proto_send_data(sock, dat)
}

impl TryFrom<AlarmInfo> for Alarm {
    type Error = Box<dyn std::error::Error>;
    fn try_from(value: AlarmInfo) -> Result<Self, Self::Error> {
        let t = value.time.ok_or("time cannot be absent")?;
        Ok(Self {
            title: value.title,
            description: value.desc,
            time: {
                Datetime {
                    date: None,
                    time: Some(toml::value::Time {
                        hour: max((t / 3600) as u8, 23),
                        minute: max(((t % 3600) / 60) as u8, 59),
                        second: max((t % 60) as u8, 59),
                        nanosecond: 0,
                    }),
                    offset: None,
                }
            },
            repeat: {
                if value.repeat.len() != 0 {
                    Some(value.repeat)
                } else {
                    None
                }
            },
            sound: value.sound,
            icon: value.icon,
        })
    }
}

// Failure should return InternalServerError
impl TryFrom<Alarm> for AlarmInfo {
    type Error = Box<dyn std::error::Error>;
    fn try_from(value: Alarm) -> Result<Self, Self::Error> {
        let mut ret = Self::new();
        let t = value.time.time.ok_or("alarm time cannot be none")?;
        ret.title = value.title;
        ret.desc = value.description;
        ret.time = Some(t.hour as u32 * 3600 + t.minute as u32 * 60 + t.second as u32);
        ret.repeat = value.repeat.unwrap_or(vec![]);
        ret.sound = value.sound;
        ret.icon = value.icon;
        Ok(ret)
    }
}

fn determine_entry_day(atime: toml::value::Time, rep: &Option<Vec<String>>) -> Option<NaiveDate> {
    let loc = Local::now();
    let ld = loc.date_naive();
    let t = NaiveTime::from_hms_opt(atime.hour.into(), atime.minute.into(), atime.second.into());
    if let Some(z) = t {
        find_next_rep(
            if z > loc.time() {
                ld
            } else {
                ld.succ_opt().unwrap()
            },
            rep,
        )
    } else {
        // out of legal times, but not returning Result, so return None
        None
    }
}
