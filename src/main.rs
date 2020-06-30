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

fn disk_usage(log: &slog::Logger) -> Result<f64, String> {
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
    Ok(root_mount.avail.as_u64() as f64 / root_mount.total.as_u64() as f64)
}

fn should_push(percentage: f64, treshold: f64, previous_percentage: f64) -> bool {
    if percentage >= treshold {
        return false;
    }
    // If treshold is met we want to avoid spamming unless disk usage keeps increasing
    (percentage * 100.0).floor() < (previous_percentage * 100.0).floor()
}

fn check_disk_usage(
    log: &slog::Logger,
    percentage: f64,
    treshold: f64,
    previous_percentage: f64,
) -> Result<f64, String> {
    if percentage < treshold {
        info!(log, "Low disk space treshold met");
    } else {
        debug!(log, "Low disk space treshold not met");
    }
    if should_push(percentage, treshold, previous_percentage) {
        push(&log, percentage)?;
    }
    Ok(percentage)
}

fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let root = slog::Logger::root(drain, o!());
    let treshold = 0.1;
    let sleep_time = Duration::from_secs(60 * 5);

    let mut previous_percentage = 1.0;

    info!(root, "Starting disk-warn");
    let log = root.new(o!("treshold" => treshold, "sleep_time" => sleep_time.as_secs()));
    loop {
        let log = log.new(o!("previous" => format!("{:.2}", previous_percentage)));
        let percentage = match disk_usage(&log) {
            Ok(percentage) => percentage,
            Err(err) => {
                error!(log, "Got error while calculating disk usage: {:#?}", err);
                continue;
            }
        };
        let log = log.new(o!("percentage" => format!("{:.2}", percentage)));
        match check_disk_usage(&log, percentage, treshold, previous_percentage) {
            Ok(percentage) => {
                previous_percentage = percentage;
            }
            Err(err) => {
                error!(log, "Got error while checking disk usage: {:#?}", err);
            }
        }
        thread::sleep(sleep_time);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_should_push_initial_below_treshold() {
        let percentage = 0.09;
        let previous = 1.0;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(
            push,
            "Push should be true when below treshold and previous above treshold"
        );
    }

    #[test]
    fn test_should_not_push_initial_above_treshold() {
        let percentage = 0.20;
        let previous = 0.3;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(
            !push,
            "Push should be true when below treshold and previous above treshold"
        );
    }

    #[test]
    fn test_should_not_push_unchanged() {
        let percentage = 0.09;
        let previous = 0.09;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(!push, "Push should be false when there is no change");
    }

    #[test]
    fn test_should_not_push_less_than_one_percent() {
        let percentage = 0.09;
        let previous = 0.09;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(
            !push,
            "Push should be false when less than one percent has changed"
        );
    }

    #[test]
    fn test_should_push_more_than_one_percent() {
        let percentage = 0.08;
        let previous = 0.09;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(
            push,
            "Push should be false when less than one percent has changed"
        );
    }

    #[test]
    fn test_should_not_push_increasing() {
        let percentage = 0.09;
        let previous = 0.08;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(
            !push,
            "Push should be false when less than one percent has changed"
        );
    }

    #[test]
    fn test_should_not_push_increase_above_treshold() {
        let percentage = 0.11;
        let previous = 0.09;
        let treshold = 0.1;

        let push = should_push(percentage, treshold, previous);

        assert!(
            !push,
            "Push should be false when less than one percent has changed"
        );
    }
}
