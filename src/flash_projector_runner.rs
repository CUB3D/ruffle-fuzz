///! Support for running a fuzz case under flash projector and gathering output

use std::io::Read;
use std::time::{Duration, Instant};
use rand::{RngCore, SeedableRng};
use subprocess::{Exec, Redirection};
use crate::{DELETE_SWF, FLASH_PLAYER_BINARY, MyError};

pub async fn open_flash_cmd(bytes: Vec<u8>) -> Result<(String, Duration), MyError> {
    let flash_start = Instant::now();

    // let mut log_path = dirs_next::config_dir().expect("No config dir");
    // log_path.push(FLASH_LOG_PATH);

    // let _ = OpenOptions::new()
    //     .write(true)
    //     .truncate(true)
    //     .open(&log_path)?;

    let path = format!("./run/test-{}.swf", rand::rngs::SmallRng::from_entropy().next_u32());
    std::fs::write(&path, bytes)?;

    let cmd = Exec::cmd(FLASH_PLAYER_BINARY)
        .env("LD_PRELOAD", "./utils/path-mapping.so")
        // .env("DISPLAY", ":2")
        .args(&[path.clone()])
        .stderr(Redirection::File(std::fs::File::open("/dev/null").unwrap()))
        .stdout(Redirection::Pipe)
        .detached();

    let start_time = Instant::now();
    let mut popen = cmd.popen()?;

    let mut log_content = "".to_string();

    loop {
        popen
            .stdout
            .as_mut()
            .unwrap()
            .read_to_string(&mut log_content)?;

        if log_content.contains("#CASE_COMPLETE#") {
            break;
        }

        if Instant::now().duration_since(start_time) > Duration::from_secs(30) {
            println!("Flash timed out, run > 30s");
            break;
        }

        if let Ok(Some(ex)) = popen.wait_timeout(Duration::from_millis(100)) {
            if !ex.success() {
                tracing::info!("Flash crashed with {:?}", ex);
                if DELETE_SWF {
                    std::fs::remove_file(&path)?;
                }
                return Err(MyError::FlashCrash);
            } else {
                break;
            }
        }
    }

    popen.kill()?;
    popen.terminate()?;
    drop(popen);

    if DELETE_SWF {
        std::fs::remove_file(&path)?;
    }

    Ok((log_content, Instant::now() - flash_start))
}
