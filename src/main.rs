use crate::swf_generator::SwfGenerator;
use env_logger::Env;
use md5::Digest;
use ruffle_core::backend::audio::NullAudioBackend;
use ruffle_core::backend::log::LogBackend;
use ruffle_core::backend::navigator::NullNavigatorBackend;
use ruffle_core::backend::storage::MemoryStorageBackend;
use ruffle_core::backend::ui::NullUiBackend;
use ruffle_core::backend::video::NullVideoBackend;
use ruffle_core::tag_utils::SwfMovie;
use ruffle_render::backend::null::NullRenderer;
use ruffle_render::backend::ViewportDimensions;
use std::cell::RefCell;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use subprocess::{Exec, Redirection};
use rand::SeedableRng;
use rand::RngCore;
use crate::error::MyError;
use crate::flash_projector_runner::open_flash_cmd;
use crate::fuzz_session::{fuzz, SharedFuzzState};

pub mod failure_checker;
pub mod rng;
pub mod swf_generator;
pub mod error;
pub mod flash_projector_runner;
pub mod ruffle_runner;
pub mod fuzz_session;

///*Note*: Only 1 of these should be enabled at a time
/// Should single opcode fuzz cases be generated
const OPCODE_FUZZ: bool = false;
/// Should static function fuzz cases be generated
const STATIC_FUNCTION_FUZZ: bool = false;
/// Should dynamic function fuzz cases be generated, (function calls on an objet/other value)
const DYNAMIC_FUNCTION_FUZZ: bool = true;

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

/// Should threads be pinned to cores
const PIN_THREADS: bool = true;

/// Should low level timeing info be collected, like the time for running the file in each player
pub const TIMING_DEBUG: bool = false;

/// Should only a single iteration be performed
pub const SINGLE_ITER: bool = false;

/// Should the input be removed after running a test
pub const DELETE_SWF: bool = false;

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

    //TODO: setup mm.cfg

    tracing::info!("Starting fuzz loop");

    let state = Arc::new(SharedFuzzState::default());

    let stats_state = Arc::clone(&state);
    std::thread::spawn(move || loop {
        let iters = stats_state.iterations.load(Ordering::SeqCst);
        stats_state
            .total_iterations
            .fetch_add(iters, Ordering::SeqCst);
        stats_state.iterations.store(0, Ordering::SeqCst);
        let total_iters = stats_state.total_iterations.load(Ordering::SeqCst);

        tracing::info!("Iterations = {}, iters/s = {}", total_iters, iters / 5,);
        std::thread::sleep(Duration::from_secs(5));
    });

    // Create thread for each fuzzing job
    let threads = (0..THREAD_COUNT)
        .map(|thread_index| {
            let state_copy = Arc::clone(&state);
            std::thread::spawn(move || {
                if PIN_THREADS {
                    // Attempt to pin threads to cores on linux
                    #[cfg(target_os = "linux")]
                    {
                        let tid = unsafe { libc::pthread_self() };

                        let mut cpu_set: libc::cpu_set_t = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
                        unsafe { libc::CPU_ZERO(&mut cpu_set) };
                        unsafe { libc::CPU_SET(thread_index as usize, &mut cpu_set) };

                        unsafe { libc::sched_setaffinity(tid as i32, core::mem::size_of::<libc::cpu_set_t>(), &cpu_set) };
                    }
                }

                // Start fuzzing
                fuzz(state_copy).expect("Thread failed");
            })
        })
        .collect::<Vec<_>>();
    for x in threads {
        x.join().expect("Thread failed to join or panic");
    }

    Ok(())
}

// Write the opcodes to a file as well
//TODO:
// Dynamic function more classes
// Try using Class.prototype.func() with the wrong `this` arg
// avm2 support
// registers and slots and movieclips as value types
