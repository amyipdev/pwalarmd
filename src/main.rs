use std::{path::Path, io::BufReader, fs::File};

use colored::Colorize;
use rodio::{OutputStream, Decoder, Source};
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
    sound: Option<String>
}

#[derive(Serialize, Deserialize)]
struct Alarm {
    title: Option<String>,
    description: Option<String>,
    time: Datetime,
    // ["Mo", "We", ...]
    repeat: Option<Vec<String>>,
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
    let testfile = BufReader::new(File::open("assets/hyper-alarm.mp3").unwrap());
    let src = Decoder::new(testfile).unwrap();
    stream_handle.play_raw(src.convert_samples());
    std::thread::sleep(std::time::Duration::from_secs(16));

    // Model for alarms: VecDeque
    // Instead of checking that we're at the exact time for an alarm,
    // We see if we're past it, then push it backwards
    // Do check that the cached date is correct though - handle system sleep, don't want to throw tons of alarms

    Ok(())
}

fn beprint(msg: &str) {
    eprint!("{}", "pwalarmd".yellow().bold());
    eprintln!(": {}", msg.bright_red());
}
