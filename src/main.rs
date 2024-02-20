// TODO: better error handling
// The only thing that should actually crash is config issues
// (or extraordinary circumstances, like unrepresentable times)
// Otherwise, fall back to a sane default + log
// TODO: pwalarmctl
// send requests (new, modify, remove); save to write to .toml
//   (if and only if user has write access)
use std::{
    cmp::Ordering, collections::VecDeque, fs::File, io::BufReader, path::Path, time::Duration,
};

use chrono::{Datelike, Local, NaiveDate, NaiveTime, Weekday};
use colored::Colorize;
use notify_rust::Notification;
use rodio::{source::SamplesConverter, Decoder, OutputStream, Source};
use serde_derive::{Deserialize, Serialize};
use toml::value::Datetime;

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
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Alarm {
    title: Option<String>,
    description: Option<String>,
    time: Datetime,
    // ["Mo", "We", ...]
    repeat: Option<Vec<String>>,
    // TODO: allow Volume control, sets system volume (avoids mute)
    sound: Option<String>,
    // TODO: allow icon setting for notifications
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

// Packaging config:
// One in /etc/pwalarmd.toml
// One in /etc/xdg/pwalarmd/pwalarmd.sample.toml
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shellex = shellexpand::tilde("~/config/pwalarmd/pwalarmd.toml").to_string();
    // TODO: make this mut for pwalarmctl
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
    // TODO: mutable configs
    let config: Config = match toml::from_str(match std::fs::read_to_string(&config_path) {
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

    // TODO: better error handling
    // TODO: dynamic config change
    // NOTE: This current state is simply a basic test. This is not the actual app.
    // Need to make the alarm loop still, and then daemonize.
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    //let testfile = BufReader::new(File::open("assets/hyper-alarm.mp3").unwrap());
    /*let global_sound = Decoder::new(BufReader::new(File::open(
        config
            .general
            .sound
            .unwrap_or("assets/hyper-alarm.mp3".to_string()),
    )?))?
    .convert_samples::<f32>();*/
    let global_sound = config
        .general
        .sound
        .unwrap_or("assets/hyper-alarm.mp3".to_string());

    // Model for alarms: VecDeque
    // Instead of checking that we're at the exact time for an alarm,
    // We see if we're past it, then push it backwards
    // Do check that the cached date is correct though - handle system sleep,
    // don't want to throw tons of alarms

    // Set up initial alarm list
    let mut alarm_ring: VecDeque<LocalAlarm> = VecDeque::new();
    if let Some(alarms) = config.alarms {
        for alarm in alarms {
            let atime = alarm.time.time.unwrap();
            let loc = Local::now();
            let ld = loc.date_naive();
            let la = LocalAlarm {
                next_run_date: if NaiveTime::from_hms_opt(
                    atime.hour.into(),
                    atime.minute.into(),
                    atime.second.into(),
                )
                .ok_or("no legal times")?
                    > loc.time()
                {
                    match find_next_rep(ld, &alarm.repeat) {
                        Some(v) => v,
                        None => continue,
                    }
                } else {
                    match find_next_rep(ld.succ_opt().unwrap(), &alarm.repeat) {
                        Some(v) => v,
                        None => continue,
                    }
                },
                alarm,
            };
            alarm_ring.insert(alarm_ring.binary_search(&la).unwrap_or_else(|e| e), la);
        }
    }

    let polltime = if let Some(t) = config.general.poll {
        t
    } else {
        250
    };
    // Processing loop
    // TODO: daemonize before entering loop
    loop {
        std::thread::sleep(Duration::from_millis(polltime));
        // TODO: check config file for changes
        // TODO: check unix socket for new pwalarmctl requests
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
                noti.appname(if let Some(ref s) = config.general.custom_app_name {
                    s
                } else {
                    "pwalarmd"
                });
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
