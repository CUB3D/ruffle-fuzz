use std::error::Error;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use md5::Digest;
use crate::{FAILURES_DIR, MyError, open_flash_cmd, SINGLE_ITER, SwfGenerator, TIMING_DEBUG};
use crate::ruffle_runner::open_ruffle;

/// The fuzz state shared between threads
#[derive(Default)]
pub struct SharedFuzzState {
    /// All of the files that we have tested so far
    attempted: RwLock<Vec<Digest>>,

    pub iterations: AtomicUsize,
    pub total_iterations: AtomicUsize,
}

pub fn fuzz(shared_state: Arc<SharedFuzzState>) -> Result<(), Box<dyn Error>> {
    let mut overall_duration = Duration::ZERO;
    let mut ruffle_duration = Duration::ZERO;
    let mut flash_duration = Duration::ZERO;
    let mut iters = 0;
    let mut swf_content = Vec::with_capacity(1024);
    let mut swf_generator = SwfGenerator::new();

    loop {
        let start = Instant::now();
        // Keep generating until we produce a unique swf
        let mut warning_shown = false;
        let swf_md5 = loop {
            swf_content.clear();

            swf_generator.reset();
            swf_generator.next_swf(&mut swf_content)?;
            let swf_md5 = md5::compute(&swf_content);
            // If its unique
            if !shared_state.attempted.read().unwrap().contains(&swf_md5) {
                // Store it
                shared_state.attempted.write().unwrap().push(swf_md5);
                break swf_md5;
            }
            if Instant::now().duration_since(start) > Duration::from_secs(10) && !warning_shown {
                tracing::info!("No unique swfs generated in 10 seconds, are we done?");
                warning_shown = true;
            }
            if Instant::now().duration_since(start) > Duration::from_secs(30) {
                tracing::info!("No unique swfs generated in 30 seconds, killing thread");
                return Ok(());
            }
        };

        let (ruffle_result, flash_result) = futures::executor::block_on(async {
            let ruffle_res = open_ruffle(swf_content.clone()).await;
            let flash_res = open_flash_cmd(swf_content.clone()).await;

            (ruffle_res, flash_res)
        });

        let (flash_res, flash_dur) = match flash_result {
            Ok(x) => Ok(x),
            Err(MyError::FlashCrash) => {
                tracing::info!("Flash crash detected, ignoring input");
                continue;
            }
            Err(e) => Err(e),
        }?;
        if TIMING_DEBUG {
            flash_duration += flash_dur;
        }

        let (ruffle_res, ruffle_dur) = ruffle_result?;
        if TIMING_DEBUG {
            ruffle_duration += ruffle_dur;
        }

        // Did we find a mismatch
        if ruffle_res != flash_res {
            let new_name = format!("{:x}", swf_md5);
            tracing::info!("Found mismatch @ {}", new_name);
            let specific_failure_dir = PathBuf::from_str(FAILURES_DIR)
                .expect("No failures-other dir")
                .join(new_name);

            let _ = std::fs::create_dir(&specific_failure_dir);

            std::fs::write(&specific_failure_dir.join("out.swf"), &swf_content)?;
            std::fs::write(&specific_failure_dir.join("ruffle.txt"), ruffle_res)?;
            std::fs::write(&specific_failure_dir.join("flash.txt"), flash_res)?;
        }

        if SINGLE_ITER {
            std::process::exit(0);
        }

        if TIMING_DEBUG {
            overall_duration += Instant::now() - start;
            iters += 1;
        }
        shared_state.iterations.fetch_add(1, Ordering::SeqCst);

        if TIMING_DEBUG && overall_duration > Duration::from_secs(1) {
            tracing::info!(
                "Iter/s = {}, duration = {:?}, ruffle={:?}, flash={:?}",
                iters,
                overall_duration / iters,
                ruffle_duration / iters,
                flash_duration / iters
            );
            overall_duration = Duration::ZERO;
            ruffle_duration = Duration::ZERO;
            flash_duration = Duration::ZERO;
            iters = 0;
        }
    }
}
