use env_logger::Env;
use md5::Digest;
use rand::rngs::SmallRng;
use rand::{Rng, RngCore, SeedableRng, thread_rng};
use ruffle_core::backend::audio::NullAudioBackend;
use ruffle_core::backend::log::LogBackend;
use ruffle_core::backend::navigator::NullNavigatorBackend;
use ruffle_core::backend::render::NullRenderer;
use ruffle_core::backend::storage::MemoryStorageBackend;
use ruffle_core::backend::ui::NullUiBackend;
use ruffle_core::backend::video::NullVideoBackend;
use ruffle_core::tag_utils::SwfMovie;
use std::cell::RefCell;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use log::{info, warn};
use ruffle_core::external::{ExternalInterfaceMethod, ExternalInterfaceProvider};
use subprocess::{Exec, Redirection};
use swf::avm1::types::{Action, Value};
use swf::{Compression, Header, Rectangle, SwfStr, Tag, Twips};
use thiserror::Error;
use tokio::task::JoinError;
use crate::rng::XorShift;
use crate::swf_generator::SwfGenerator;

pub mod failure_checker;
pub mod ptrace_fuzz;
pub mod rng;
pub mod swf_generator;

///*Note*: Only 1 of these should be enabled at a time
/// Should single opcode fuzz cases be generated
const OPCODE_FUZZ: bool = true;
/// Should static function fuzz cases be generated
const STATIC_FUNCTION_FUZZ: bool = false;
/// Should dynamic function fuzz cases be generated, (function calls on an objet/other value)
const DYNAMIC_FUNCTION_FUZZ: bool = false;

#[cfg(windows)]
const INPUTS_DIR: &str = ".\\run\\inputs";
#[cfg(windows)]
const FAILURES_DIR: &str = ".\\run\\failures";
#[cfg(windows)]
const FLASH_PLAYER_BINARY: &str = ".\\utils\\flashplayer_32_sa_debug.exe";
#[cfg(windows)]
const FLASH_LOG_PATH: &str = "Macromedia\\Flash Player\\Logs\\flashlog.txt";

#[cfg(unix)]
const INPUTS_DIR: &str = "./run/inputs/";
#[cfg(unix)]
const FAILURES_DIR: &str = "./run/failures/";
#[cfg(unix)]
const FLASH_PLAYER_BINARY: &str = "./utils/flashplayer_32_sa_debug";
// const FLASH_PLAYER_BINARY: &str = "./utils/flashplayer_10_3r183_90_linux_sa";
#[cfg(unix)]
const FLASH_LOG_PATH: &str = "../.macromedia/Flash_Player/Logs/flashlog.txt";

/// Generate random byte-strings, otherwise use fixed value string ("This is a test")
const FUZZ_RANDOM_STRING: bool = false;

/// Generate random numbers, otherwise use fixed value numbers (10)
const FUZZ_RANDOM_INT: bool = false;

/// Generate strings with ints, otherwise use fixed strings
const FUZZ_INT_STRING: bool = false;

/// Generate NaN doubles
const FUZZ_DOUBLE_NAN: bool = false;

/// Use random swf versions, otherwise only use 32 (latest)
const RANDOM_SWF_VERSION: bool = false;

/// Number of threads to use
const THREAD_COUNT: i32 = 1;

/// Should low level timeing info be collected, like the time for running the file in each player
pub const TIMING_DEBUG: bool = false;

#[derive(Error, Debug)]
enum MyError {
    #[error("Flash Crash")]
    FlashCrash,

    #[error("Io Error")]
    IoError(#[from] std::io::Error),

    #[error("Popen Error")]
    PopenError(#[from] subprocess::PopenError),

    #[error("Join error")]
    JoinError(#[from] JoinError),
}

async fn open_flash_cmd(bytes: Vec<u8>) -> Result<(String, Duration), MyError> {
    let flash_start = Instant::now();

    // let mut log_path = dirs_next::config_dir().expect("No config dir");
    // log_path.push(FLASH_LOG_PATH);

    // let _ = OpenOptions::new()
    //     .write(true)
    //     .truncate(true)
    //     .open(&log_path)?;

    let path = format!("./run/test-{}.swf", SmallRng::from_entropy().next_u32());
    tokio::fs::write(&path, bytes).await?;

    let cmd = Exec::cmd(FLASH_PLAYER_BINARY)
        .env("LD_PRELOAD", "./utils/path-mapping.so")
        // .env("DISPLAY", ":2")
        .args(&[path.clone()])
        .stderr(Redirection::File(std::fs::File::open("/dev/null").unwrap()))
        .stdout(Redirection::Pipe)
        .detached();

    let mut popen = cmd.popen()?;

    let mut log_content = "".to_string();

    loop {
        popen
            .stdout
            .as_mut()
            .unwrap()
            .read_to_string(&mut log_content)?;
        // log_content = std::fs::read_to_string(&log_path)?;
        // tracing::info!("{}", log_content);
        if log_content.contains("#CASE_COMPLETE#") {
            break;
        }

        if let Ok(Some(ex)) = popen.wait_timeout(Duration::from_millis(100)) {
            if !ex.success() {
                tracing::info!("Flash crashed with {:?}", ex);
                tokio::fs::remove_file(&path).await?;
                return Err(MyError::FlashCrash);
            } else {
                break;
            }
        }
    }

    popen.kill()?;
    popen.terminate()?;
    drop(popen);

    tokio::fs::remove_file(&path).await?;

    Ok((log_content, Instant::now() - flash_start))
}

#[derive(Default)]
struct StringLogger {
    msgs: RefCell<String>,
}

impl LogBackend for StringLogger {
    fn avm_trace(&self, message: &str) {
        let mut st = self.msgs.borrow_mut();
        st.push_str(message);
        st.push('\n');
    }
    fn __fuzz__get_log_string(&self) -> String {
        self.msgs.borrow().to_string()
    }
}

async fn open_ruffle(bytes: Vec<u8>) -> Result<(String, Duration), MyError> {
    let ruffle_start = Instant::now();

    let movie = SwfMovie::from_data(&bytes, None, None).expect("Load movie fail");
    let log = StringLogger::default();

    let player = ruffle_core::PlayerBuilder::new()
        .with_renderer(NullRenderer::default())
        .with_audio(NullAudioBackend::default())
        .with_navigator(NullNavigatorBackend::default())
        .with_storage(MemoryStorageBackend::default())
        .with_video(NullVideoBackend::default())
        .with_log(log)
        .with_ui(NullUiBackend::new())
        .build();


    let mut lock = player.lock().unwrap();
    lock.set_root_movie(movie);
    lock.set_is_playing(true);
    drop(lock);


    loop {
        let mut lock = player.lock().unwrap();

        lock.run_frame();
        lock.tick(1000. / 60.);
        lock.render();
        if !lock.is_playing() {
            break;
        }

        let out = lock.log_backend().__fuzz__get_log_string();
        if out.contains("#CASE_") {
            lock.set_is_playing(false);
        }
    }

    let lock = player.lock().unwrap();
    let out = lock.log_backend().__fuzz__get_log_string();
    Ok((out, Instant::now() - ruffle_start))
}

fn fuzz(shared_state: Arc<SharedFuzzState>) -> Result<(), Box<dyn Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .worker_threads(1)
        .build()
        .unwrap();

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

        let local = tokio::task::LocalSet::new();

        let (ruffle_future, flash_future) = local.block_on(&rt, async {
            let ruffle_future = tokio::task::spawn_local(open_ruffle(swf_content.clone()));
            let flash_future = tokio::task::spawn_local(open_flash_cmd(swf_content.clone()));

            tokio::join!(ruffle_future, flash_future)
        });
        let (ruffle_result, flash_result) = (ruffle_future?, flash_future?);

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
            tracing::info!("Found mismatch");
            let new_name = format!("{:x}", swf_md5);
            let specific_failure_dir = PathBuf::from_str(FAILURES_DIR)
                .expect("No failures-other dir")
                .join(new_name);

            let _ = std::fs::create_dir(&specific_failure_dir);

            rt.block_on(tokio::fs::write(
                &specific_failure_dir.join("out.swf"),
                &swf_content,
            ))?;
            rt.block_on(tokio::fs::write(
                &specific_failure_dir.join("ruffle.txt"),
                ruffle_res,
            ))?;
            rt.block_on(tokio::fs::write(
                &specific_failure_dir.join("flash.txt"),
                flash_res,
            ))?;
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

/// Empty the flash log file, this avoids a crash were the file is missing
fn clear_flash_log() -> Result<(), Box<dyn Error>> {
    let log_path = dirs_next::config_dir()
        .expect("No config dir")
        .join(FLASH_LOG_PATH);
    let mut flash_log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)?;
    flash_log.write_all(&[])?;
    Ok(())
}

/// The fuzz state shared between threads
#[derive(Default)]
struct SharedFuzzState {
    /// All of the files that we have tested so far
    attempted: RwLock<Vec<Digest>>,

    pub iterations: AtomicUsize,
    pub total_iterations: AtomicUsize,
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("flash_fuzz=info")).init();

    // create the run dir
    std::fs::create_dir_all(FAILURES_DIR)?;
    std::fs::create_dir_all(INPUTS_DIR)?;
    // Create the flash dir
    let flash_log = dirs_next::config_dir()
        .expect("No config dir")
        .join(FLASH_LOG_PATH);
    std::fs::create_dir_all(flash_log.parent().unwrap())?;
    // Ensure that the flash log exists or we will crash
    clear_flash_log()?;

    // Start dedicated X server for fuzzing on linux TODO
    // #[cfg(target_os = "linux")]
    //     let _ = {
    //     Exec::cmd("sudo")
    //         .args(&["echo"])
    //         .join().expect("Failed to elevate")
    // };
    // #[cfg(target_os = "linux")]
    //     let popen = {
    //         Exec::cmd("sudo")
    //             .args(&["Xorg", ":2", "-config", "./docker-fuzz/xorg.conf", "-noreset", "-logfile", "/dev/null"])
    //             .popen().expect("Failed to start Xorg server for fuzzing")
    //     };

    //TODO: setup mm.cfg

    tracing::info!("Starting fuzz loop");

    let state = Arc::new(SharedFuzzState::default());

    let stats_state = Arc::clone(&state);
    std::thread::spawn(move || {
        loop {
            let iters = stats_state.iterations.load(Ordering::SeqCst);
            stats_state
                .total_iterations
                .fetch_add(iters, Ordering::SeqCst);
            stats_state.iterations.store(0, Ordering::SeqCst);
            let total_iters = stats_state.total_iterations.load(Ordering::SeqCst);

            tracing::info!("Iterations = {}, iters/s = {}", total_iters, iters / 5,);
            std::thread::sleep(Duration::from_secs(5));
        }
    });

    // Create thread for each fuzzing job
    let threads = (0..THREAD_COUNT)
        .map(|_thread_index| {
            let state_copy = Arc::clone(&state);
            std::thread::spawn(move || {
                // Attempt to pin threads to cores on linux (dissabled for now)
                // #[cfg(target_os = "linux")]
                //     {
                //         let tid = unsafe { libc::pthread_self() };
                //
                //         let mut cpu_set: libc::cpu_set_t = unsafe { MaybeUninit::zeroed().assume_init() };
                //         unsafe { libc::CPU_ZERO(&mut cpu_set) };
                //         unsafe { libc::CPU_SET(thread_index as usize, &mut cpu_set) };
                //
                //         unsafe { libc::sched_setaffinity(tid as i32, core::mem::size_of::<libc::cpu_set_t>(), &cpu_set) };
                //     }

                // Start fuzzing
                fuzz(state_copy).expect("Thread failed");
            })
        })
        .collect::<Vec<_>>();
    for x in threads {
        x.join().expect("Thread failed to join or panic");
    }

    // let _ = fuzz().await?;
    // check_failures();

    Ok(())
}

// Write the opcodes to a file as well
