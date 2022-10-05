use std::fmt::format;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::thread::sleep;
use chrono::prelude::*;
use std::process::{Command, Stdio};
use chrono::Duration;
use log::{error, info};
use serde::{Deserialize, Serialize};

const DAILY_TIMELAPSE_SHOT_INTERVAL: u32 = 2; // shoot daily every 2 minutes
const LONG_TERM_TIMELAPSE_SHOT_HOUR: u32 = 12; // shoot longterm at noon

const DAILY_TIMELAPSE_SHOTS_DIR: &str = "/home/pi/Timelapse/Daily/TempPhotos";
const LONG_TERM_TIMELAPSE_SHOTS_DIR: &str = "/home/pi/Timelapse/LongTerm/TempPhotos";
const DAILY_TIMELAPSE_VIDS_DIR: &str = "/home/pi/Timelapse/Daily/Videos";
const LONG_TERM_TIMELAPSE_VIDS_DIR: &str = "/home/pi/Timelapse/LongTerm/Videos";

const STATE_FILE_PATH: &str = "/home/pi/Timelapse/state.json";

#[derive(Serialize, Deserialize)]
struct State {
    last_daily_capture: DateTime<Local>,
    last_longterm_capture: DateTime<Local>,
    last_daily_video: DateTime<Local>,
    last_longterm_video: DateTime<Local>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_daily_capture: Local::now(),
            last_longterm_capture: Local::now(),
            last_daily_video: Local::now(),
            last_longterm_video: Local::now(),
        }
    }
}

fn main() {
    setup_logger().unwrap();

    fs::create_dir_all(DAILY_TIMELAPSE_SHOTS_DIR).unwrap();
    fs::create_dir_all(LONG_TERM_TIMELAPSE_SHOTS_DIR).unwrap();
    fs::create_dir_all(DAILY_TIMELAPSE_VIDS_DIR).unwrap();
    fs::create_dir_all(LONG_TERM_TIMELAPSE_VIDS_DIR).unwrap();

    let mut curr_state = load_curr_state();

    loop {
        let curr_time = Local::now();

        // Compile vids

        // used for file names
        let yesterday = curr_time - Duration::days(1);

        if curr_time.day() != curr_state.last_daily_video.day() {
            let filename = format!("daily-{}.mp4", yesterday.format("%Y-%m-%d").to_string());
            compile_vid(&filename, DAILY_TIMELAPSE_SHOTS_DIR, DAILY_TIMELAPSE_VIDS_DIR, 25);
            clean_up_dir(DAILY_TIMELAPSE_SHOTS_DIR);
            curr_state.last_daily_video = curr_time;
        }

        if curr_time.month() != curr_state.last_longterm_video.month() {
            let filename = format!("longterm-{}.mp4", yesterday.format("%Y-%m").to_string());
            compile_vid(&filename, LONG_TERM_TIMELAPSE_SHOTS_DIR, LONG_TERM_TIMELAPSE_VIDS_DIR, 15);
            clean_up_dir(LONG_TERM_TIMELAPSE_SHOTS_DIR);
            curr_state.last_longterm_video = curr_time;
        }

        // Capture shots

        if curr_time.minute() % DAILY_TIMELAPSE_SHOT_INTERVAL == 0 && curr_state.last_daily_capture.minute() != curr_time.minute() {
            let filename = format!("daily-{}.jpg", curr_time.format("%Y-%m-%d_%H%M").to_string());
            capture(&filename, DAILY_TIMELAPSE_SHOTS_DIR);
            curr_state.last_daily_capture = curr_time;
        }

        if curr_time.hour() == LONG_TERM_TIMELAPSE_SHOT_HOUR && curr_time.day() != curr_state.last_longterm_capture.day() {
            let filename = format!("longterm-{}.jpg", curr_time.format("%Y-%m-%d").to_string());
            capture(&filename, LONG_TERM_TIMELAPSE_SHOTS_DIR);
            curr_state.last_longterm_capture = curr_time;
        }

        save_curr_state(&curr_state);

        // Sleep for 1 min

        sleep(std::time::Duration::from_secs(60));
    }
}

fn load_curr_state() -> State {
    let file = fs::read_to_string(STATE_FILE_PATH);
    if file.is_err() {
        State::default()
    } else {
        let contents = file.unwrap();
        serde_json::from_str(&contents).unwrap()
    }
}

fn save_curr_state(state: &State) {
    let serialized = serde_json::to_string(state).unwrap();
    fs::write(STATE_FILE_PATH, serialized).unwrap();
}

fn capture(filename: &str, output_dir: &str) {
    let output = Command::new("libcamera-jpeg")
        .arg("-o")
        .arg(Path::join(output_dir.as_ref(), filename))
        .output();

    if output.is_err() {
        error!("libcamera-jpeg failed attempting to capture a shot.");
    }
}

fn compile_vid(filename: &str, source_dir: &str, output_dir: &str, fps: i32) {
    info!("compile vid: filename: {}; srcdir: {}; outputdir: {}; fps: {}", filename, source_dir, output_dir, fps);

    let result = Command::new("video-fromimg")
        .args(["--input-files", &format!("{}/*.jpg", source_dir), "--fps", &fps.to_string(), Path::join(output_dir.as_ref(), filename).to_str().unwrap()])
        .output();

    if result.is_err() {
        error!("failed to compile");
    } else {
        info!("compiled successfully");
    }
}

fn clean_up_dir(dir: &str) {
    for entry in fs::read_dir(dir).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
}

fn setup_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .chain(fern::log_file("output.log")?)
        .apply()?;
    Ok(())
}
