use rand::{Rng, SeedableRng, RngCore};
use std::error::Error;
use std::fs::OpenOptions;
use std::time::{Duration, Instant};
use subprocess::{Exec, Redirection};
use swf::avm1::types::{Action, Value};
use swf::{Compression, Header, Rectangle, SwfStr, Tag, Twips};
use rand::rngs::SmallRng;
use thiserror::Error;
use std::io::{Write, Read};
use tokio::task::JoinError;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;
use ruffle_core::backend::render::NullRenderer;
use ruffle_core::backend::audio::NullAudioBackend;
use ruffle_core::backend::navigator::NullNavigatorBackend;
use ruffle_core::backend::storage::MemoryStorageBackend;
use ruffle_core::backend::locale::NullLocaleBackend;
use ruffle_core::backend::video::NullVideoBackend;
use ruffle_core::backend::log::LogBackend;
use ruffle_core::backend::ui::NullUiBackend;
use ruffle_core::tag_utils::SwfMovie;
use std::sync::{Arc, Mutex};
use std::cell::RefCell;
use std::mem::MaybeUninit;
use env_logger::Env;
use md5::Digest;

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
const FLASH_PLAYER_BINARY: & str = "./utils/flashplayer_32_sa_debug";
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
const RANDOM_SWF_VERSION:bool =false;

/// Number of threads to use
const THREAD_COUNT: i32 = 8;


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


/// Create a new random test case, will return Ok(bytes) on success or Err(_) on error
fn make_swf() -> Result<Vec<u8>, Box<dyn Error>> {
    // let mut rng = rand::thread_rng();
    let mut rng = SmallRng::from_entropy();

    // common swf stuff
    //TODO: versions < 6 seem to hang the official player? maybe some opcodes aren't implemented? We could just add a timeout?
    let swf_version: u8 = if RANDOM_SWF_VERSION { rng.gen_range(6..=32) } else {32};
    let swf_header: Header = Header {
        compression: Compression::None,
        version: swf_version,
        stage_size: Rectangle {
            x_min: Twips::from_pixels(0.),
            y_min: Twips::from_pixels(0.),
            x_max: Twips::from_pixels(10.),
            y_max: Twips::from_pixels(10.),
        },
        frame_rate: 60.into(),
        num_frames: 0,
    };

    let mut strings = Vec::with_capacity(10);

    // Define the main code
    let mut do_action_bytes = Vec::with_capacity(1024);
    use swf::avm1::write::Writer;
    let mut w = Writer::new(&mut do_action_bytes, swf_version);

    // Log entire stack
    fn dump_stack(w: &mut Writer<&mut Vec<u8>>) -> Result<(), Box<dyn Error>> {
        let pos = w.output.len();
        w.write_action(&Action::PushDuplicate)?;
        w.write_action(&Action::Trace)?;
        w.write_action(&Action::Push(vec![Value::Str("#PREFIX#".into())]))?;
        w.write_action(&Action::Equals2)?;
        w.write_action(&Action::Not)?;
        let offset = pos.wrapping_sub(w.output.len());
        w.write_action(&Action::If {
            offset: offset as i16 - 5
        })?;

        Ok(())
    }

    // Generate a random value with random contents
    fn random_value<'val, 'strings: 'val>(rng: &mut rand::rngs::SmallRng, strings: &'strings mut Vec<Vec<u8>>) -> Value<'val>{
        match rng.gen_range(0..=6) {
            0 => Value::Undefined,
            1 => Value::Null,
            2 => Value::Int(if FUZZ_RANDOM_INT { rng.gen() } else { 10 }),
            3 => Value::Bool(rng.gen()),
            //TODO: double are also known to not match
            4 => {
                if FUZZ_DOUBLE_NAN {
                    match rng.gen_range(0..=1) {
                        0 => Value::Double(if FUZZ_RANDOM_INT { rng.gen::<i64>() as f64 } else { 10. }),
                        1 => Value::Double(f64::NAN /*rng.gen()*/),
                        _ => unreachable!()
                    }
                } else {
                    Value::Double(if FUZZ_RANDOM_INT { rng.gen::<i64>() as f64 } else { 10. })
                }
            },
            //TODO: floats are known to not match in ruffle
            5 => Value::Float(f32::NAN/*rng.gen()*/),
            6 => {
                if FUZZ_INT_STRING {
                    // Decide if we should make a text, or numerical string
                    match rng.gen_range(0..=1) {
                        0 => {
                            if FUZZ_RANDOM_STRING {
                                // Completely random bytes for strings
                                let max_string_len = 256;
                                let mut buf = Vec::<u8>::with_capacity(max_string_len);
                                buf.resize(rng.gen_range(1..max_string_len), 0);
                                rng.fill(buf.as_mut_slice());
                                strings.push(buf);
                            } else {
                                strings.push("this is a test".as_bytes().to_vec())
                            }
                        }
                        // Generate a integer numerical string
                        1 => {
                            let v = if FUZZ_RANDOM_INT { rng.gen::<i32>() } else { 10 };
                            strings.push(v.to_string().into_bytes());
                        }
                        // Generate a decimal numerical string
                        //TODO: dissabled as it can cause issues with some functions(yes that is a bug in the functions (guessing a unnessicary cast to float causing float mismatching) but its so common it makes spotting other issues hard)
                        //TODO: dont forget to increase range above
                        // 2 => {
                        //     let v = rng.gen::<f32>();
                        //     strings.push(v.to_string().into_bytes());
                        // }
                        _ => unreachable!()
                    }
                } else {
                    strings.push("this is a test".as_bytes().to_vec())
                }

                Value::Str(SwfStr::from_bytes(strings.last().unwrap().as_slice()))
            }
            _ => unreachable!()
        }
    }

    fn random_value_simple<'val, 'strings: 'val>(rng: &mut rand::rngs::SmallRng, strings: &'strings mut Vec<Vec<u8>>) -> Value<'val> {
        match rng.gen_range(0..=6) {
            0 => Value::Undefined,
            1 => Value::Null,
            2 => Value::Int(10),
            3 => Value::Bool(rng.gen()),
            4 => Value::Double(10.),
            5 => Value::Float(10.),
            6 => {
                strings.push("this is a test".as_bytes().to_vec());
                Value::Str(SwfStr::from_bytes(strings.last().unwrap().as_slice()))
            }
            _ => unreachable!()
        }
    }

    fn select<T: Clone>(rng: &mut SmallRng, options: &[T]) -> T {
        let index= rng.gen_range(0..options.len());
        options[index].clone()
    }

    // Put something on the stack so if the add produces nothing, we get a known value
    w.write_action(&Action::Push(vec![Value::Str("#PREFIX#".into())]))?;

    if DYNAMIC_FUNCTION_FUZZ {
        //TODO: support for flash.foo.bar.Thing
        //TODO: looks like ruffle has a bug where flash.geom.Point can be referenced as just Point, hmm maybe try fuzzing for that
        let classes: &[(&str, RangeInclusive<i32>, &[&str], &[(&str, &[&str])])] = &[
            /*("Point", 2..=2, &["length", "x", "y"], &[
                ("add", &["Point"])
            ]),*/
            ("String", 1..=1, &["length"], &[
                ("charAt", &["Number"])
            ]),
            // Array actually has no arg limit, but we still want a reasonable chance of the 0/1 arg case as they are special
            ("Array", 0..=10, &["length"], &[
                ("concat", &["Array"]),
                ("join", &["Array"]),
                ("pop", &[]),
                ("push", &["Any"]),
                ("reverse", &[]),
                ("shift", &[]),
                ("slice", &["Number", "Number"]),
                ("sort", &["Number", "Number"]),
                ("sortOn", &["Number", "Number"]),
                ("splice", &["Number", "Number", "Number"]),
                ("toString", &[]),
                ("unshift", &["Number"]),
            ])
        ];

        //TODO: should we fuzz the case of args/classes to
        let (class_name, constructor_arg_range, properties, functions) = select(&mut rng, classes);
        //Ignore this, for same reason as in static
        let arg_count = rng.gen_range(0..=*constructor_arg_range.end());

        // The name of the object
        strings.push("foo".as_bytes().to_vec());
        w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(strings.last().unwrap()))]))?;

        // Push the args
        for _ in 0..arg_count {
            w.write_action(&Action::Push(vec![random_value(&mut rng, &mut strings)]))?;
        }

        // The name, the arg count
        strings.push(class_name.as_bytes().to_vec());
        w.write_action(&Action::Push(vec![Value::Int(arg_count), Value::Str(SwfStr::from_bytes(strings.last().unwrap()))]))?;
        //TODO: some use newmethod
        w.write_action(&Action::NewObject)?;
        w.write_action(&Action::DefineLocal)?;

        // Pick a random function
        let (function_name, args) = select(&mut rng, functions);
        let function_arg_count = rng.gen_range(0..=args.len() as i32);

        // Push function args and arg count
        for _ in 0..function_arg_count {
            w.write_action(&Action::Push(vec![random_value(&mut rng, &mut strings)]))?;
        }
        w.write_action(&Action::Push(vec![Value::Int(function_arg_count)]))?;

        // Get foo
        strings.push("foo".as_bytes().to_vec());
        w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(strings.last().unwrap()))]))?;
        w.write_action(&Action::GetVariable)?;

        // Call foo.<function_name>()
        strings.push(function_name.as_bytes().to_vec());
        w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(strings.last().unwrap()))]))?;
        w.write_action(&Action::CallMethod)?;

        let _ = dump_stack(&mut w)?;

        //TODO: dump return val + all properties
        //TODO: run multiple functions on each object
        //TODO: pay attention to types of args
    }

    //TODO: we need a way to generate objects, e.g point
    if STATIC_FUNCTION_FUZZ {
        let static_methods = &[
            ("Accessibility", "isActive", 0..=0),
            ("BitmapData", "loadBitmap", 1..=1),
            ("Camera", "get", 0..=1),
            ("CustomActions", "get", 1..=1),
            ("CustomActions", "install", 2..=2),
            ("CustomActions", "list", 0..=0),
            ("CustomActions", "uninstall", 1..=1),
            ("Date", "UTC", 2..=7),
            ("ExternalInterface", "addCallback", 3..=3),
            ("ExternalInterface", "call", 1..=2),
            //IME
            ("Key", "getAscii", 0..=0),
            ("Key", "getCode", 0..=0),
            ("Key", "isAccessible", 0..=0),
            ("Key", "isDown", 1..=1),
            ("Key", "isToggled", 1..=1),
            ("Key", "removeListener", 1..=1),
            ("Locale", "checkXMLStatus", 0..=0),
            ("Locale", "getDefaultLang", 0..=0),
            ("Locale", "loadString", 1..=1),
            ("Locale", "loadStringEx", 2..=2),
            ("String", "fromCharCode", 1..=1),
            //Math
            ("Microphone", "get", 1..=1),
            ("Mouse", "hide", 0..=0),
            ("Mouse", "removeListener", 1..=1),
            ("Mouse", "show", 0..=0),
            ("Object", "registerClass", 2..=2),
            ("Point", "distance", 2..=2),
            ("Point", "interpolate", 3..=3),
            ("Point", "polar", 2..=2),
            ("Selection", "getBeginIndex", 0..=0),
            ("Selection", "getCaretIndex", 0..=0),
            ("Selection", "getEndIndex", 0..=0),
            ("Selection", "getFocus", 0..=0),
            ("Selection", "removeListener", 1..=1),
            ("Selection", "setFocus", 1..=1),
            ("SharedObject", "getLocal", 1..=3),
            ("Stage", "removeListener", 1..=1),
            ("TextField", "getFontList", 0..=0),
            ("XMLUI", "get", 1..=1),
        ];

        let (obj_name, func_name, arg_count_range) = select(&mut rng, static_methods);
        // Some functions take a variable argument counts, pick a random number of args to get good coverage
        // We ignore the lower bound here as we also want to test how missing args are handled in avm1
        // In avm2 we will want to make use of that, as missing args will cause exceptions
        let arg_count = rng.gen_range(0..=*arg_count_range.end());

        for _ in 0..arg_count {
            w.write_action(&Action::Push(vec![random_value(&mut rng, &mut strings)]))?;
        }

        w.write_action(&Action::Push(vec![Value::Int(arg_count), Value::Str(obj_name.into())]))?;
        w.write_action(&Action::GetVariable)?;
        w.write_action(&Action::Push(vec![Value::Str(func_name.into())]))?;
        w.write_action(&Action::CallMethod)?;

        dump_stack(&mut w)?;
    }

    if OPCODE_FUZZ {
        //TODO: ActionAdd produces errors in some cases
        // todo: so does less
        let (action, arg_count) = select(&mut rng, &[
            // (Action::Add, 2),
   /*         (Action::Add2, 2),
            (Action::And, 2),
            (Action::AsciiToChar, 1),
            (Action::BitAnd, 2),
            (Action::BitLShift, 2),
            (Action::BitOr, 2),
            (Action::BitRShift, 2),
            (Action::BitURShift, 2),
            (Action::BitXor, 2),
            //_
            (Action::CastOp, 2),
            (Action::CharToAscii, 1),
            //_
            // TODO: constant pool
            (Action::Decrement, 1),
            //_
            // TODO: divide
            (Action::Enumerate, 1),
            (Action::Enumerate2, 1),
            (Action::Equals, 2),
            (Action::Equals2, 2),
            //_
            (Action::Greater, 2),
            // (Action::ImplementsOp, ?), TODO: needs special handling
            (Action::Increment, 1),
            // (Action::InitArray, ?), TODO: special handling
            // (Action::InitObject, ?), TODO: special handling
            (Action::InstanceOf, 2),*/
            (Action::Less, 2),
      /*      (Action::Less2, 2),
            (Action::MBAsciiToChar, 1),
            (Action::MBCharToAscii, 1),
            (Action::MBStringExtract, 3),
            (Action::MBStringLength, 1),
            // (Action::Modulo, 2), TODO: doubles dont match
            // (Action::Multiply, 2), TODO: doubles dont match
            //_
            (Action::Not, 1),
            (Action::Or, 2),
            //_
            (Action::Pop, 1),
            //_
            (Action::PushDuplicate, 1),
            //_
            (Action::StackSwap, 2),
            //_
            (Action::StrictEquals, 2),
            (Action::StringAdd, 2),
            (Action::StringEquals, 2),
            (Action::StringExtract, 3),
            (Action::StringGreater, 2),
            (Action::StringLength, 1),
            (Action::StringLess, 2),
            // (Action::Subtract, 2), TODO: doubles dont match
            (Action::TargetPath, 1),
            //_
            (Action::ToInteger, 1),
            (Action::ToNumber, 1),
            (Action::ToString, 1),
            (Action::ToggleQuality, 0),
            (Action::Trace, 1),
            (Action::TypeOf, 1),*/
            //_
        ]);

        //TODO: rest of non-frame actions
        //TODO: dump entire stack, not just top so we can check multi value actions like enumerate

        for _ in 0..arg_count {
            w.write_action(&Action::Push(vec![random_value_simple(&mut rng, &mut strings)]))?;
        }
        // Testing arithmetic ops
        w.write_action(&action)?;

        let _ = dump_stack(&mut w)?;
    }

    // Log a sentinal so we know that its done
    w.write_action(&Action::Push(vec![Value::Str("#CASE_COMPLETE#".into())]))?;
    w.write_action(&Action::Trace)?;

    w.write_action(&Action::GetUrl {target: "_root".into(), url: "fscommand:quit".into()})?;

    let mut output = Vec::with_capacity(1000);

    // Create the swf
    swf::write_swf(
        &swf_header,
        &[
            Tag::DoAction(do_action_bytes.as_slice()),
            Tag::EnableDebugger(SwfStr::from_utf8_str("$1$5C$2dKTbwjNlJlNSvp9qvD651")),
        ],
        &mut output,
    )?;


    Ok(output)
}

/// Use the linux `ptrace` API to inject swfs and hook log file writes, this allows running multiple flash instances in parallel
/// and improves perf by avoiding file system writes
async fn open_flash_ptrace(bytes: &[u8]) -> Result<(String, Duration), MyError> {
    let flash_start = Instant::now();

    let process_path = "./utils/flashplayer_32_sa_debug";
    let process_name = "flashplayer_32_sa_debug";
    let arg = "./test.swf";
    let mut ptrace = ptrace::Ptrace::new(process_path, process_name, arg).unwrap();
    ptrace.vfs_mut().mock_file(&["./test.swf", "/mnt/Media/torrent/flash-fuzz/./test.swf"], bytes.to_vec());
    ptrace.vfs_mut().mock_file(&["/home/cub3d/.macromedia/Flash_Player/Logs/flashlog.txt"], vec![0u8]);

    ptrace.spawn(Box::new(|_pt, event| {
        tracing::info!("Got event {:?}", event);
    }));

    let log_bytes = ptrace.vfs_mut().get_file_content_by_path("/home/cub3d/.macromedia/Flash_Player/Logs/flashlog.txt").unwrap();
    if log_bytes == [0] {
        panic!();
    }
    let log_content = String::from_utf8(log_bytes).unwrap();
    Ok((log_content, Instant::now() - flash_start))
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
        .args(&[path.clone()])
        .stderr(Redirection::File(std::fs::File::open("/dev/null").unwrap()))
        .stdout(Redirection::Pipe)
        .detached();

    let mut popen = cmd.popen()?;

    let mut log_content= "".to_string();

    loop {
        popen.stdout.as_mut().unwrap().read_to_string(&mut log_content)?;
        // log_content = std::fs::read_to_string(&log_path)?;
        // tracing::info!("{}", log_content);
        if log_content.contains("#CASE_COMPLETE#") {
            break;
        }

        if let Ok(Some(ex)) = popen.wait_timeout(Duration::from_millis(10)) {
            if !ex.success() {
                tracing::info!("Flash crashed with {:?}", ex);
                tokio::fs::remove_file(&path).await?;
                return Err(MyError::FlashCrash)
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
    msgs: RefCell<String>
}

impl LogBackend for StringLogger {
    fn avm_trace(&self, message: &str) {
        self.msgs.borrow_mut().push_str(&format!("{}\n", message.to_string()));
    }
}

async fn open_ruffle(bytes: Vec<u8>) -> Result<(String, Duration), MyError> {
    let ruffle_start = Instant::now();

    let movie = SwfMovie::from_data(&bytes, None, None).expect("Load movie fail");
    let log = Box::new(StringLogger::default());
    let player = ruffle_core::Player::new(Box::new(NullRenderer::default()), Box::new(NullAudioBackend::default()), Box::new(NullNavigatorBackend::default()), Box::new(MemoryStorageBackend::default()), Box::new(NullLocaleBackend::default()), Box::new(NullVideoBackend::default()), log, Box::new(NullUiBackend::new())).expect("Failed to mk player");
    let mut lock = player.lock().unwrap();
    lock.set_root_movie(Arc::new(movie));
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

        let lb = lock.log_backend().downcast_ref::<StringLogger>().unwrap();
        let out = lb.msgs.clone().borrow().clone();
        if out.contains("#CASE_") {
            lock.set_is_playing(false);
        }
    }

    let lock = player.lock().unwrap();
    let lb = lock.log_backend().downcast_ref::<StringLogger>().unwrap();
    let out = lb.msgs.clone().borrow().clone();
    Ok((out, Instant::now() - ruffle_start))
}

fn fuzz(shared_state: Arc<Mutex<SharedFuzzState>>) -> Result<(), Box<dyn Error>> {
    let rt = tokio::runtime::Builder::new_current_thread().worker_threads(1).build().unwrap();

    let mut overall_duration = Duration::ZERO;
    let mut ruffle_duration = Duration::ZERO;
    let mut flash_duration = Duration::ZERO;
    let mut iters = 0;
    loop {
        let start = Instant::now();
        // Keep generating until we produce a unique swf
        let mut warning_shown = false;
        let (swf_content, swf_md5) = loop {
            let swf_content = make_swf()?;
            let swf_md5 = md5::compute(&swf_content);
            // If its unique
            let mut shared_state = shared_state.lock().unwrap();
            if !shared_state.attempted.contains(&swf_md5) {
                // Store it
               shared_state.attempted.push(swf_md5);
                break (swf_content, swf_md5);
            }
            if Instant::now().duration_since(start) > Duration::from_secs(30) && !warning_shown {
                tracing::info!("No unique swfs generated in 30 seconds, are we done?");
                warning_shown = true;
            }
            if Instant::now().duration_since(start) > Duration::from_secs(120) {
                tracing::info!("No unique swfs generated in 120 seconds, killing thread");
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
            Err(e) => Err(e)
        }?;
        flash_duration += flash_dur;

        let (ruffle_res, ruffle_dur) = ruffle_result?;
        ruffle_duration += ruffle_dur;

        // Did we find a mismatch
        if ruffle_res != flash_res {
            tracing::info!("Found mismatch");
            let new_name = format!("{:x}", swf_md5);
            let specific_failure_dir = PathBuf::from_str(FAILURES_DIR).expect("No failures-other dir").join(new_name);

            let _ = std::fs::create_dir(&specific_failure_dir);

            rt.block_on(tokio::fs::write(&specific_failure_dir.join("out.swf"), &swf_content))?;
            rt.block_on(tokio::fs::write(&specific_failure_dir.join("ruffle.txt"), ruffle_res))?;
            rt.block_on(tokio::fs::write(&specific_failure_dir.join("flash.txt"), flash_res))?;
        }

        overall_duration += Instant::now() - start;
        iters += 1;

        if overall_duration > Duration::from_secs(1) {
            tracing::info!("iters/s = {}, duration = {:?}, ruffle={:?}, flash={:?}", iters, overall_duration / iters, ruffle_duration/iters, flash_duration/iters);
            overall_duration = Duration::ZERO;
            ruffle_duration = Duration::ZERO;
            flash_duration = Duration::ZERO;
            iters = 0;
        }
    }
}

async fn check_failures() -> Result<(), Box<dyn Error>> {
    let dir = std::fs::read_dir(FAILURES_DIR)?;

    let mut total = 0;
    let mut failed = 0;

    for entry in dir.flatten().filter(|e| e.file_type().is_ok()).filter(|e| e.file_type().unwrap().is_dir()) {
        let swf_path = entry.path().join("out.swf");
        let flash_output_path = entry.path().join("flash.txt");
        let swf_content = tokio::fs::read(swf_path).await?;

        //TODO:
        let (ruffle_res, _) = open_ruffle(swf_content).await?;
        let expected = std::fs::read_to_string(flash_output_path.to_str().unwrap())?;

        if ruffle_res != expected {
            tracing::info!("---------- Found mismatch ----------");
            tracing::info!("Test case = {}", entry.file_name().to_string_lossy());
            tracing::info!("Ruffle output:");
            tracing::info!("{}", ruffle_res);
            tracing::info!("Flash output:");
            tracing::info!("{}", expected);
            tracing::info!("------------------------------------");
            failed += 1;
        } else {
            tracing::info!("Test case {} - Passed", entry.file_name().to_string_lossy());
        }
        total += 1;
    }

    tracing::info!("Overall results: {}/{} failed", failed, total);

    Ok(())
}

/// Empty the flash log file, this avoids a crash were the file is missing
fn clear_flash_log() -> Result<(), Box<dyn Error>> {
    let log_path = dirs_next::config_dir().expect("No config dir").join(FLASH_LOG_PATH);
    let mut flash_log = OpenOptions::new().create(true).write(true).truncate(true).open(log_path)?;
    flash_log.write_all(&[])?;
    Ok(())
}

/// The fuzz state shared between threads
#[derive(Default)]
struct SharedFuzzState {
    /// All of the files that we have tested so far
    attempted: Vec<Digest>
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("flash_fuzz=info")).init();

    // create the run dir
    std::fs::create_dir_all(FAILURES_DIR)?;
    std::fs::create_dir_all(INPUTS_DIR)?;
    // Create the flash dir
    let flash_log = dirs_next::config_dir().expect("No config dir").join(FLASH_LOG_PATH);
    std::fs::create_dir_all(flash_log.parent().unwrap())?;
    // Ensure that the flash log exists or we will crash
    let _ = clear_flash_log()?;

    //TODO: setup mm.cfg

    tracing::info!("Starting fuzz loop");

    let state = Arc::new(Mutex::new(SharedFuzzState::default()));

    // Create thread for each fuzzing job
    let threads = (0..THREAD_COUNT).map(|thread_index| {
        let state_copy = Arc::clone(&state);
        std::thread::spawn(move || {
            let tid = unsafe { libc::pthread_self() };
            // let mut bs = bitset::BitSet::with_capacity(64);
            // bs.set(thread_index as usize, true);

            let mut cpu_set: libc::cpu_set_t = unsafe { MaybeUninit::zeroed().assume_init() };
            unsafe { libc::CPU_ZERO(&mut cpu_set) };
            unsafe { libc::CPU_SET(thread_index as usize, &mut cpu_set) };

            unsafe { libc::sched_setaffinity(tid as i32, core::mem::size_of::<libc::cpu_set_t>(), &cpu_set) };

            // Start fuzzing
            let _ = fuzz(state_copy).expect("Thread failed");
        })
    }).collect::<Vec<_>>();
    for x in threads {
        x.join().expect("Thread failed to join or panic");
    }

    // let _ = fuzz().await?;
    // check_failures();

    Ok(())
}
