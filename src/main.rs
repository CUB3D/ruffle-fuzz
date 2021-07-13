use rand::Rng;
use regex::Regex;
use std::error::Error;
use std::fs::OpenOptions;
use std::time::{Duration, Instant};
use subprocess::{Exec, Redirection};
use swf::avm1::types::{Action, Value};
use swf::{Compression, Header, Rectangle, SwfStr, Tag, Twips};
use rand::rngs::ThreadRng;
use thiserror::Error;
use std::io::Write;
use std::thread::JoinHandle;
use tokio::task::JoinError;
use std::ops::RangeInclusive;

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

    #[error("Failed to build ruffle")]
    RuffleBuild,
}


/// Create a new random test case, will return Ok(Some(path)) on success or Err(_) on error
fn make_swf() -> Result<String, Box<dyn Error>> {
    let mut rng = rand::thread_rng();

    // common swf stuff
    //TODO: versions < 6 seem to hang the player? maybe some opcodes aren't implemented? We could just add a timeout?
    let swf_version: u8 = rng.gen_range(6..=32);
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

    let mut strings = Vec::new();

    // Define the main code
    let mut do_action_bytes = Vec::new();
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
    fn random_value<'val, 'strings: 'val>(rng: &mut ThreadRng, strings: &'strings mut Vec<Vec<u8>>) -> Value<'val>{
        match rng.gen_range(0..=7) {
            0 => Value::Undefined,
            1 => Value::Null,
            2 => Value::Int(rng.gen()),
            3 => Value::Bool(rng.gen()),
            //TODO: double are also known to not match
            4 => Value::Double(f64::NAN /*rng.gen()*/),
            5 => Value::Double(rng.gen::<i64>() as f64),
            //TODO: floats are known to not match in ruffle
            6 => Value::Float(f32::NAN/*rng.gen()*/),
            7 => {
                // Decide if we should make a text, or numerical string
                match rng.gen_range(0..=1) {
                    0 => {
                        let random_strings = false;

                        if random_strings {
                            // Completely random bytes for strings
                            let mut buf = Vec::<u8>::new();
                            let max_string_len = 256;
                            buf.resize(rng.gen_range(1..max_string_len), 0);
                            rng.fill(buf.as_mut_slice());
                            strings.push(buf);
                        } else {
                            strings.push("this is a test".as_bytes().to_vec())
                        }
                    }
                    // Generate a integer numerical string
                    1 => {
                        let v = rng.gen::<i32>();
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


                Value::Str(SwfStr::from_bytes(strings.last().unwrap().as_slice()))
            }
            _ => unreachable!()
        }
    }

    fn select<T: Clone>(rng: &mut ThreadRng, options: &[T]) -> T {
        let index= rng.gen_range(0..options.len());
        options[index].clone()
    }

    // Put something on the stack so if the add produces nothing, we get a known value
    w.write_action(&Action::Push(vec![Value::Str("#PREFIX#".into())]))?;

    const OPCODE_FUZZ: bool = false;
    const STATIC_FUNCTION_FUZZ: bool = false;
    const DYNAMIC_FUNCTION_FUZZ: bool = true;

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
        w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(&strings.last().unwrap()))]))?;

        // Push the args
        for _ in 0..arg_count {
            w.write_action(&Action::Push(vec![random_value(&mut rng, &mut strings)]))?;
        }

        // The name, the arg count
        strings.push(class_name.as_bytes().to_vec());
        w.write_action(&Action::Push(vec![Value::Int(arg_count), Value::Str(SwfStr::from_bytes(&strings.last().unwrap()))]))?;
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
        w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(&strings.last().unwrap()))]))?;
        w.write_action(&Action::GetVariable)?;

        // Call foo.<function_name>()
        strings.push(function_name.as_bytes().to_vec());
        w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(&strings.last().unwrap()))]))?;
        w.write_action(&Action::CallMethod)?;

        dump_stack(&mut w);

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
            (Action::Add2, 2),
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
            (Action::InstanceOf, 2),
            (Action::Less, 2),
            (Action::Less2, 2),
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
            (Action::TypeOf, 1),
            //_
        ]);

        //TODO: rest of non-frame actions
        //TODO: dump entire stack, not just top so we can check multi value actions like enumerate

        for _ in 0..arg_count {
            w.write_action(&Action::Push(vec![random_value(&mut rng, &mut strings)]))?;
        }
        // Testing arithmetic ops
        w.write_action(&action)?;

        let _ = dump_stack(&mut w)?;
    }

    // Log a sentinal so we know that its done
    w.write_action(&Action::Push(vec![Value::Str("#CASE_COMPLETE#".into())]))?;
    w.write_action(&Action::Trace)?;

    // Create the swf
    let path = ".\\run\\inputs\\out.swf".to_string();
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;

    swf::write_swf(
        &swf_header,
        &[
            Tag::DoAction(do_action_bytes.as_slice()),
            Tag::EnableDebugger(SwfStr::from_utf8_str("$1$5C$2dKTbwjNlJlNSvp9qvD651")),
        ],
        &mut output,
    )?;

    Ok(path)
}

async fn open_flash(path: String) -> Result<(String, Duration), MyError> {
    let flash_start = Instant::now();

    let mut log_path = dirs_next::config_dir().expect("No config dir");
    log_path.push("Macromedia\\Flash Player\\Logs\\flashlog.txt");

    let _ = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&log_path)?;

    let cmd = Exec::cmd("utils\\flashplayer_32_sa_debug.exe")
        .args(&[path])
        // .stdin(Redirection::Pipe)
        .stdout(Redirection::Pipe)
        .detached();

    let mut popen = cmd.popen()?;

    let mut log_content;

    loop {
        log_content = std::fs::read_to_string(&log_path)?;
        if log_content.contains("#CASE_COMPLETE#") {
            break;
        }

        if let Ok(Some(_ex)) = popen.wait_timeout(Duration::from_millis(10)) {
            return Err(MyError::FlashCrash)
        }
    }

    popen.kill()?;
    popen.terminate()?;

    Ok((log_content, Instant::now() - flash_start))
}

lazy_static::lazy_static! {
    static ref RUFFLE_LOG_REGEX: Regex = Regex::new(r#"\[[0123456789\-T:Z]+ INFO {2}ruffle_core::backend::log] "#).unwrap();
}

async fn open_ruffle(path: String) -> Result<(String, Duration), MyError> {
    let ruffle_start = Instant::now();

    let ruffle_log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .read(true)
        .open(".\\run\\ruffle-log.txt")?;

    let ruffle_path_utils = ".\\utils\\ruffle_desktop.exe";
    let ruffle_path_direct = ".\\ruffle\\target\\release\\ruffle_desktop.exe";

    let cmd = Exec::cmd(ruffle_path_direct)
        .env_extend(&[("RUST_BACKTRACE", "full"), ("RUST_LOG", "avm_trace=debug")])
        .args(&[&path])
        // .stdin(Redirection::Pipe)
        // .stderr(Redirection::Pipe)
        .stdout(Redirection::File(ruffle_log))
        .detached();

    let mut popen = cmd.popen()?;

    let mut log_content;

    loop {
        log_content = std::fs::read_to_string(".\\run\\ruffle-log.txt")?;
        if log_content.contains("#CASE_COMPLETE#") {
            break;
        }

        if let Ok(Some(_ex)) = popen.wait_timeout(Duration::from_millis(10)) {
            println!("Failed to start {:?}", path);
            break;
        }
    }

    // TODO: currently requires setting env_log to write to stderr

    let out = RUFFLE_LOG_REGEX.replace_all(&log_content, "").replace("\n", "\r\n");

    popen.kill()?;
    popen.terminate()?;

    Ok((out, Instant::now() - ruffle_start))
}

async fn run_flash_ruffle_parallel(path: String) -> Result<(Result<(String, Duration), MyError>, Result<(String, Duration), MyError>), MyError> {
    let ruffle_future = tokio::spawn(open_ruffle(path.clone()));
    let flash_future = tokio::spawn(open_flash(path));

    let (ruffle_result, flash_result) = tokio::join!(ruffle_future, flash_future);
    Ok((ruffle_result?, flash_result?))
}

async fn fuzz() -> Result<(), Box<dyn Error>> {
    let mut overall_duration = Duration::ZERO;
    let mut ruffle_duration = Duration::ZERO;
    let mut flash_duration = Duration::ZERO;
    let mut iters = 0;
    loop {
        let start = Instant::now();
        let path = make_swf()?;

        let (ruffle_result, flash_result) = run_flash_ruffle_parallel(path.to_string()).await?;

        let (flash_res, flash_dur) = match flash_result {
            Ok(x) => Ok(x),
            Err(MyError::FlashCrash) => {
                println!("Flash crash detected, ignoring input");
                std::fs::remove_file(&path)?;
                continue;
            }
            Err(e) => Err(e)
        }?;
        flash_duration += flash_dur;

        let (ruffle_res, ruffle_dur) = ruffle_result?;
        ruffle_duration += ruffle_dur;

        // Did we find a mismatch
        if ruffle_res != flash_res {
            println!("Found mismatch");
            let swf_content = std::fs::read(&path)?;
            let new_name = format!("{:x}", md5::compute(&swf_content));
            let _ = std::fs::create_dir(&format!(".\\run\\failures\\{}", new_name));
            let _ = std::fs::rename(&path, &format!(".\\run\\failures\\{}\\out.swf", new_name));
            let _ = std::fs::write(&format!(".\\run\\failures\\{}\\ruffle.txt", new_name), ruffle_res);
            let _ = std::fs::write(&format!(".\\run\\failures\\{}\\flash.txt", new_name), flash_res);
        } else {
            // If its the same, remove the test
            std::fs::remove_file(&path)?;
        }

        overall_duration += Instant::now() - start;
        iters += 1;

        if overall_duration > Duration::from_secs(1) {
            println!("iters/s = {}, duration = {:?}, ruffle={:?}, flash={:?}", iters, overall_duration / iters, ruffle_duration/iters, flash_duration/iters);
            overall_duration = Duration::ZERO;
            ruffle_duration = Duration::ZERO;
            flash_duration = Duration::ZERO;
            iters = 0;
        }
    }
}

async fn check_failures() -> Result<(), Box<dyn Error>> {
    let dir = std::fs::read_dir(".\\run\\failures")?;

    let mut total = 0;
    let mut failed = 0;

    for entry in dir.flatten().filter(|e| e.file_type().is_ok()).filter(|e| e.file_type().unwrap().is_dir()) {
        let swf_path = entry.path().join(".\\out.swf");
        let flash_output_path = entry.path().join(".\\flash.txt");

        let (ruffle_res, _) = open_ruffle(swf_path.to_str().unwrap().to_string()).await?;
        let expected = std::fs::read_to_string(flash_output_path.to_str().unwrap())?;

        if ruffle_res != expected {
            println!("---------- Found mismatch ----------");
            println!("Test case = {}", entry.file_name().to_string_lossy());
            println!("Ruffle output:");
            println!("{}", ruffle_res);
            println!("Flash output:");
            println!("{}", expected);
            println!("------------------------------------");
            failed += 1;
        } else {
            println!("Test case {} - Passed", entry.file_name().to_string_lossy());
        }
        total += 1;
    }

    println!("Overall results: {}/{} failed", failed, total);

    Ok(())
}

fn rebuild_ruffle() -> Result<(), Box<dyn Error>> {
    println!("Rebuilding ruffle");
    let res = Exec::cmd("cargo")
        .args(&["build", "--release"])
        .cwd(".\\ruffle\\desktop")
        .join()?;
    if res.success() {
        Ok(())
    } else {
        Err(MyError::RuffleBuild.into())
    }
}

/// Empty the flash log file, this avoids a crash were the file is missing
fn clear_flash_log() -> Result<(), Box<dyn Error>> {
    let mut log_path = dirs_next::config_dir().expect("No config dir");
    log_path.push("Macromedia\\Flash Player\\Logs\\flashlog.txt");
    let mut flash_log = OpenOptions::new().create(true).write(true).truncate(true).open(log_path)?;
    flash_log.write_all(&[])?;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads=4)]
async fn main() -> Result<(), Box<dyn Error>> {
    // create the run dir
    std::fs::create_dir_all(".\\run\\failures")?;
    std::fs::create_dir_all(".\\run\\inputs")?;
    // Ensure that the flash log exists or we will crash
    let _ = clear_flash_log()?;

    //TODO: setup mm.cfg

    let _ = rebuild_ruffle()?;

    let _ = fuzz().await?;
    // check_failures();

    Ok(())
}
