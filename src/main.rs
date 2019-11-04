use reqwest::StatusCode;
use serde::Serialize;
use slog::Drain;
use slog::{debug, error, info, o};
use std::thread;
use std::time::Duration;
use systemstat::{Platform, System};

#[derive(Serialize, Debug)]
struct Message {
    body: String,
    title: &'static str,
    r#type: &'static str,
}

fn push(log: &slog::Logger, percentage: f64) -> Result<(), String> {
    info!(log, "Sending push message");
    let token = std::env::var("PUSHBULLET_TOKEN")
        .map_err(|err| format!("Unable to get PushBullet token: {:#?}", err))?;

    let message = Message {
        body: format!("Only {:.2}% left!", percentage * 100.0),
        title: "Low disk space",
        r#type: "note",
    };

    let client = reqwest::blocking::Client::new();
    let req = client
        .post("https://api.pushbullet.com/v2/pushes")
        .header("Access-Token", token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::to_string(&message)
                .map_err(|err| format!("Unable to serialize message: {:#?}", err))?,
        );
    let res = req
        .send()
        .map_err(|err| format!("Unable to send push message: {:#?}", err))?;
    match res.status() {
        StatusCode::OK => {
            info!(log, "Successfully sent push message");
            Ok(())
        }
        e => Err(format!("Got error from PushBullet: {:#?}", e)),
    }
}

fn check_disk_usage(log: &slog::Logger, treshold: f64, pushed: bool) -> Result<bool, String> {
    debug!(log, "Checking disk usage");
    let sys = System::new();
    let root_mount = sys
        .mounts()
        .map_err(|err| format!("Sys mount error: {:#?}", err))
        .and_then(|mounts| {
            mounts
                .into_iter()
                .find(|mount| mount.fs_mounted_on == "/")
                .ok_or_else(|| "Unable to find root mount".to_owned())
        })?;
    let percentage = root_mount.avail.as_u64() as f64 / root_mount.total.as_u64() as f64;
    let log = log.new(o!("percentage" => format!("{:.2}", percentage)));
    if percentage < treshold {
        info!(log, "Low disk space treshold met");
        if !pushed {
            push(&log, percentage)?;
        }
    } else {
        debug!(log, "Low disk space treshold not met");
    }
    Ok(percentage < treshold)
}

fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let root = slog::Logger::root(drain, o!());
    let treshold = 0.1;
    let sleep_time = Duration::from_secs(60 * 5);

    let mut pushed = false;

    info!(root, "Starting disk-warn");
    let disk_log = root.new(o!("treshold" => treshold, "sleep_time" => sleep_time.as_secs()));
    loop {
        match check_disk_usage(&disk_log, treshold, pushed) {
            Ok(warned) => {
                pushed = warned;
            }
            Err(err) => {
                error!(disk_log, "Got error while checking disk usage: {:#?}", err);
            }
        }
        thread::sleep(sleep_time);
    }
}
