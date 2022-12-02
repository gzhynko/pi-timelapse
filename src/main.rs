use std::collections::vec_deque::VecDeque;
use std::fs;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::thread::sleep;
use chrono::prelude::*;
use std::process::{Command};
use chrono::Duration;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use ssh2::Session;

const DAILY_TIMELAPSE_SHOT_INTERVAL_MINUTES: u32 = 2; // shoot daily every 2 minutes
const LONG_TERM_TIMELAPSE_SHOT_HOUR: u32 = 12; // shoot longterm at noon

const DAILY_TIMELAPSE_SHOTS_DIR: &str = "/home/pi/Timelapse/Daily/TempPhotos";
const LONG_TERM_TIMELAPSE_SHOTS_DIR: &str = "/home/pi/Timelapse/LongTerm/TempPhotos";
const DAILY_TIMELAPSE_VIDS_DIR: &str = "/home/pi/Timelapse/Daily/Videos";
const LONG_TERM_TIMELAPSE_VIDS_DIR: &str = "/home/pi/Timelapse/LongTerm/Videos";

const REMOTE_SERVER_IP: &str = "192.168.12.1";
const REMOTE_SERVER_USERNAME: &str = "gzhynko";
const REMOTE_SERVER_PATH: &str = "/home/gzhynko/Videos/Timelapse";
const PRIVATE_KEY_PATH: &str = "/home/pi/.ssh/id_rsa";

const KEEP_DAILY_TIMELAPSE_VIDEOS_DAYS: u64 = 5; // keep daily videos for 5 days
const KEEP_LONG_TERM_TIMELAPSE_VIDEOS_DAYS: u64 = 31; // keep longterm videos for a month

const STATE_FILE_PATH: &str = "/home/pi/Timelapse/state.json";

#[derive(Serialize, Deserialize)]
struct State {
    last_daily_capture: DateTime<Local>,
    last_longterm_capture: DateTime<Local>,
    last_daily_video: DateTime<Local>,
    last_longterm_video: DateTime<Local>,

    // cloud uploads
    daily_vids_to_upload: VecDeque<String>,
    longterm_vids_to_upload: VecDeque<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_daily_capture: Local::now(),
            last_longterm_capture: Local::now(),
            last_daily_video: Local::now(),
            last_longterm_video: Local::now(),

            daily_vids_to_upload: VecDeque::new(),
            longterm_vids_to_upload: VecDeque::new(),
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

        // Compile videos.

        // used for file names
        let yesterday = curr_time - Duration::days(1);

        if curr_time.day() != curr_state.last_daily_video.day() {
            let filename = format!("daily-{}.mp4", yesterday.format("%Y-%m-%d").to_string());
            compile_vid(&filename, DAILY_TIMELAPSE_SHOTS_DIR, DAILY_TIMELAPSE_VIDS_DIR, 25);
            clean_up_dir(DAILY_TIMELAPSE_SHOTS_DIR);
            curr_state.last_daily_video = curr_time;
            curr_state.daily_vids_to_upload.push_back(filename);
        }

        if curr_time.month() != curr_state.last_longterm_video.month() {
            let filename = format!("longterm-{}.mp4", yesterday.format("%Y-%m").to_string());
            compile_vid(&filename, LONG_TERM_TIMELAPSE_SHOTS_DIR, LONG_TERM_TIMELAPSE_VIDS_DIR, 15);
            clean_up_dir(LONG_TERM_TIMELAPSE_SHOTS_DIR);
            curr_state.last_longterm_video = curr_time;
            curr_state.longterm_vids_to_upload.push_back(filename);
        }

        // Capture shots.

        if curr_time.minute() % DAILY_TIMELAPSE_SHOT_INTERVAL_MINUTES == 0 && curr_state.last_daily_capture.minute() != curr_time.minute() {
            let filename = format!("daily-{}.jpg", curr_time.format("%Y-%m-%d_%H%M").to_string());
            capture(&filename, DAILY_TIMELAPSE_SHOTS_DIR);
            curr_state.last_daily_capture = curr_time;
        }

        if curr_time.hour() == LONG_TERM_TIMELAPSE_SHOT_HOUR && curr_time.day() != curr_state.last_longterm_capture.day() {
            let filename = format!("longterm-{}.jpg", curr_time.format("%Y-%m-%d").to_string());
            capture(&filename, LONG_TERM_TIMELAPSE_SHOTS_DIR);
            curr_state.last_longterm_capture = curr_time;
        }

        // Try to upload videos via SCP if there are any in the queue.

        if !curr_state.daily_vids_to_upload.is_empty() || !curr_state.longterm_vids_to_upload.is_empty() {
            let tcp_result = TcpStream::connect(format!("{}:22", REMOTE_SERVER_IP));
            if let Ok(tcp) = tcp_result {
                // Set up an ssh session and authenticate using a private key.
                let mut sess = Session::new().unwrap();
                sess.set_tcp_stream(tcp);
                sess.handshake().unwrap();
                sess.userauth_pubkey_file(REMOTE_SERVER_USERNAME, None, Path::new(PRIVATE_KEY_PATH), None).unwrap();

                // Upload daily videos
                let daily_remote_dir = format!("{}/Daily", REMOTE_SERVER_PATH);
                upload_videos(&sess, &mut curr_state.daily_vids_to_upload, DAILY_TIMELAPSE_VIDS_DIR, &daily_remote_dir);

                // Upload longterm videos
                let longterm_remote_dir = format!("{}/LongTerm", REMOTE_SERVER_PATH);
                upload_videos(&sess, &mut curr_state.longterm_vids_to_upload, LONG_TERM_TIMELAPSE_VIDS_DIR, &longterm_remote_dir);
            }
        }

        // Remove videos that are:
        // - Uploaded to the remote server (not in the upload queue).
        // - Past their max storage time.

        remove_uploaded_videos(&curr_state.daily_vids_to_upload, DAILY_TIMELAPSE_VIDS_DIR, KEEP_DAILY_TIMELAPSE_VIDEOS_DAYS);
        remove_uploaded_videos(&curr_state.longterm_vids_to_upload, LONG_TERM_TIMELAPSE_VIDS_DIR, KEEP_LONG_TERM_TIMELAPSE_VIDEOS_DAYS);

        // Save the current state to file and sleep for 1 min.

        save_curr_state(&curr_state);
        sleep(std::time::Duration::from_secs(60));
    }
}

fn upload_videos(session: &Session, vid_queue: &mut VecDeque<String>, local_vid_dir: &str, remote_vid_dir: &str) {
    // Loop through the whole queue and try to upload each video.
    // The loop is broken if any of the files failed to upload, indicating that the ssh connection might have been closed.
    while !vid_queue.is_empty() {
        let vid_filename = vid_queue.pop_front();
        if vid_filename.is_none() {
            continue;
        }
        let vid_filename = vid_filename.unwrap();

        // Access the local file.
        let local_path = Path::join(&Path::new(local_vid_dir),  &Path::new(&vid_filename));
        let local_file = File::open(local_path);
        if local_file.is_err() {
            info!("Unable to find local video file with filename {}. Removing from the upload queue.", vid_filename);
            continue;
        }
        let local_file = local_file.unwrap();

        // Copy file data to a buffer.
        let mut reader = BufReader::new(local_file);
        let mut buffer = Vec::new();
        if let Err(error) = reader.read_to_end(&mut buffer) {
            warn!("Failed to read the contents of {} to buffer: {}. Will retry its upload.", vid_filename, error);
            vid_queue.push_back(vid_filename);
            break;
        }

        info!("Starting the upload of {}.", vid_filename);
        let now = std::time::Instant::now();

        // Open a send session.
        let remote_path_str = format!("{}/{}", remote_vid_dir, vid_filename);
        let remote_path = Path::new(&remote_path_str);
        let send_result = session.scp_send(remote_path, 0o644, buffer.len() as u64, None);
        match send_result {
            Ok(_) => {}
            Err(error) => {
                warn!("Failed to open a send session for file {}: {}. Will retry its upload.", vid_filename, error);
                vid_queue.push_back(vid_filename);
                break;
            }
        }
        let mut remote_file = send_result.unwrap();

        // Try to write the buffer to the remote file.
        // In case this fails, break the loop and add the file back to the queue.
        let write_result = remote_file.write_all(&buffer);
        match write_result {
            Ok(_) => {}
            Err(error) => {
                warn!("Failed to write {} to remote file: {}. Will retry its upload.", vid_filename, error);
                vid_queue.push_back(vid_filename);
                break;
            }
        }

        // Close the channel and wait for the whole content to be transferred.

        let send_eof_result = remote_file.send_eof();
        let wait_eof_result = remote_file.wait_eof();
        if send_eof_result.is_err() || wait_eof_result.is_err() {
            warn!("send_eof or wait_eof failed while uploading {}. Will retry its upload.", vid_filename);
            vid_queue.push_back(vid_filename);
            break;
        }

        let close_result = remote_file.close();
        let wait_close_result = remote_file.wait_close();
        if close_result.is_err() || wait_close_result.is_err() {
            warn!("Failed to close upload for {}, ending the upload session. The video should have been uploaded at this point.", vid_filename);
            break;
        }

        info!("Finished the upload of {}. (took {}s)", vid_filename, now.elapsed().as_secs());
    }
}

fn remove_uploaded_videos(vid_queue: &VecDeque<String>, vid_dir: &str, allowed_storage_days: u64) {
    for entry_result in fs::read_dir(vid_dir).unwrap() {
        if let Ok(entry) = entry_result {
            let filename = entry.file_name().into_string().unwrap();
            if vid_queue.contains(&filename) {
                continue; // do not proceed if in upload queue.
            }

            if let Ok(metadata) = entry.metadata() {
                if let Ok(created_time) = metadata.created() {
                    let elapsed = created_time.elapsed().unwrap();
                    if elapsed.as_secs() / 60 / 60 / 24 > allowed_storage_days {
                        fs::remove_file(entry.path()).unwrap_or_default();
                    }
                }
            }
        }
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
    info!("Compiling video with video-fromimg: filename: {}; srcdir: {}; outputdir: {}; fps: {}.", filename, source_dir, output_dir, fps);

    let result = Command::new("video-fromimg")
        .args(["--input-files", &format!("{}/*.jpg", source_dir), "--fps", &fps.to_string(), Path::join(output_dir.as_ref(), filename).to_str().unwrap()])
        .output();

    if result.is_err() {
        error!("Failed to compile.");
    } else {
        info!("Compiled successfully.");
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
