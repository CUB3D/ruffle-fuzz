use env_logger::Env;
use md5::Digest;
use rand::rngs::SmallRng;
use rand::{Rng, RngCore, SeedableRng, thread_rng};
use ruffle_core::backend::audio::NullAudioBackend;
use ruffle_core::backend::locale::NullLocaleBackend;
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
use subprocess::{Exec, Redirection};
use swf::avm1::types::{Action, Value};
use swf::{Compression, Header, Rectangle, SwfStr, Tag, Twips};
use swf::avm1::write::Writer;
use thiserror::Error;
use tokio::task::JoinError;
use crate::{DYNAMIC_FUNCTION_FUZZ, FUZZ_DOUBLE_NAN, FUZZ_INT_STRING, FUZZ_RANDOM_INT, FUZZ_RANDOM_STRING, OPCODE_FUZZ, RANDOM_SWF_VERSION, STATIC_FUNCTION_FUZZ, XorShift};

pub struct SwfGenerator {
    rng: SmallRng,
}

impl SwfGenerator {
    pub fn new() -> Self {
        let mut rng = SmallRng::from_entropy();

        Self {
            rng
        }
    }

    /// Generate the version for the swf
    pub fn swf_version(&mut self) -> u8 {
        //TODO: versions < 6 seem to hang the official player? maybe some opcodes aren't implemented? We could just add a timeout?
        let swf_version: u8 = if RANDOM_SWF_VERSION {
            self.rng.gen_range(6..=32)
        } else {
            32
        };
        swf_version
    }

    /// Generate a swf header
    pub fn swf_header(&mut self, swf_version: u8) -> Header {
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
        swf_header
    }

    /// Select a random value from a slice
    fn select<T: Clone>(&mut self, options: &[T]) -> T {
        let index = self.rng.gen_range(0..options.len());
        options[index].clone()
    }

    pub fn random_value_simple<'val, 'strings: 'val>(
        &mut self,
        strings: &'strings mut Vec<Vec<u8>>,
    ) -> Value<'val> {
        match self.rng.gen_range(0..=6) {
            0 => Value::Undefined,
            1 => Value::Null,
            2 => Value::Int(10),
            3 => Value::Bool(self.rng.gen()),
            4 => Value::Double(10.),
            5 => Value::Float(10.),
            6 => {
                strings.push("this is a test".as_bytes().to_vec());
                Value::Str(SwfStr::from_bytes(strings.last().unwrap().as_slice()))
            }
            _ => unreachable!(),
        }
    }

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
            offset: offset as i16 - 5,
        })?;

        Ok(())
    }

    pub fn opcode_fuzz(&mut self, w: &mut Writer<&mut Vec<u8>>, strings: &mut Vec<Vec<u8>>) -> Result<(), Box<dyn Error>> {
        //TODO: ActionAdd produces errors in some cases
        // todo: so does less
        let (action, arg_count) = self.select(
            &[
                (Action::Add, 2),
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
            ],
        );

        //TODO: rest of non-frame actions
        //TODO: dump entire stack, not just top so we can check multi value actions like enumerate

        for _ in 0..arg_count {
            w.write_action(&Action::Push(vec![self.random_value_simple(
                strings,
            )]))?;
        }
        // Testing arithmetic ops
        w.write_action(&action)?;

        Self::dump_stack(w)?;

        Ok(())
    }

    /// Create a new random test case, will return Ok(()) on success or Err(_) on error
    pub fn next_swf(&mut self, output_data: &mut Vec<u8>) -> Result<(), Box<dyn Error>> {
        // let mut rng = XorShift::new(thread_rng().gen::<usize>() + 1024);

        // common swf stuff
        let swf_version = self.swf_version();
        let swf_header = self.swf_header(swf_version);

        let mut strings = Vec::with_capacity(10);

        // Define the main code
        let mut do_action_bytes = Vec::with_capacity(1024);
        use swf::avm1::write::Writer;
        let mut w = Writer::new(&mut do_action_bytes, swf_version);

        // Generate a random value with random contents
        fn random_value<'val, 'strings: 'val>(
            rng: &mut rand::rngs::SmallRng,
            strings: &'strings mut Vec<Vec<u8>>,
        ) -> Value<'val> {
            match rng.gen_range(0..=6) {
                0 => Value::Undefined,
                1 => Value::Null,
                2 => Value::Int(if FUZZ_RANDOM_INT { rng.gen() } else { 10 }),
                3 => Value::Bool(rng.gen()),
                //TODO: double are also known to not match
                4 => {
                    if FUZZ_DOUBLE_NAN {
                        match rng.gen_range(0..=1) {
                            0 => Value::Double(if FUZZ_RANDOM_INT {
                                rng.gen::<i64>() as f64
                            } else {
                                10.
                            }),
                            1 => Value::Double(f64::NAN /*rng.gen()*/),
                            _ => unreachable!(),
                        }
                    } else {
                        Value::Double(if FUZZ_RANDOM_INT {
                            rng.gen::<i64>() as f64
                        } else {
                            10.
                        })
                    }
                }
                //TODO: floats are known to not match in ruffle
                5 => Value::Float(f32::NAN /*rng.gen()*/),
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
                                let v = if FUZZ_RANDOM_INT {
                                    rng.gen::<i32>()
                                } else {
                                    10
                                };
                                strings.push(v.to_string().into_bytes());
                            }
                            // Generate a decimal numerical string
                            //TODO: dissabled as it can cause issues with some functions(yes that is a bug in the functions (guessing a unnessicary cast to float causing float mismatching) but its so common it makes spotting other issues hard)
                            //TODO: dont forget to increase range above
                            // 2 => {
                            //     let v = rng.gen::<f32>();
                            //     strings.push(v.to_string().into_bytes());
                            // }
                            _ => unreachable!(),
                        }
                    } else {
                        strings.push("this is a test".as_bytes().to_vec())
                    }

                    Value::Str(SwfStr::from_bytes(strings.last().unwrap().as_slice()))
                }
                _ => unreachable!(),
            }
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
                ("String", 1..=1, &["length"], &[("charAt", &["Number"])]),
                // Array actually has no arg limit, but we still want a reasonable chance of the 0/1 arg case as they are special
                (
                    "Array",
                    0..=10,
                    &["length"],
                    &[
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
                    ],
                ),
            ];

            //TODO: should we fuzz the case of args/classes to
            let (class_name, constructor_arg_range, _properties, functions) = self.select(classes);
            //Ignore this, for same reason as in static
            let arg_count = self.rng.gen_range(0..=*constructor_arg_range.end());

            // The name of the object
            strings.push("foo".as_bytes().to_vec());
            w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(
                strings.last().unwrap(),
            ))]))?;

            // Push the args
            for _ in 0..arg_count {
                w.write_action(&Action::Push(vec![random_value(&mut self.rng, &mut strings)]))?;
            }

            // The name, the arg count
            strings.push(class_name.as_bytes().to_vec());
            w.write_action(&Action::Push(vec![
                Value::Int(arg_count),
                Value::Str(SwfStr::from_bytes(strings.last().unwrap())),
            ]))?;
            //TODO: some use newmethod
            w.write_action(&Action::NewObject)?;
            w.write_action(&Action::DefineLocal)?;

            // Pick a random function
            let (function_name, args) = self.select(functions);
            let function_arg_count = self.rng.gen_range(0..=args.len() as i32);

            // Push function args and arg count
            for _ in 0..function_arg_count {
                w.write_action(&Action::Push(vec![random_value(&mut self.rng, &mut strings)]))?;
            }
            w.write_action(&Action::Push(vec![Value::Int(function_arg_count)]))?;

            // Get foo
            strings.push("foo".as_bytes().to_vec());
            w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(
                strings.last().unwrap(),
            ))]))?;
            w.write_action(&Action::GetVariable)?;

            // Call foo.<function_name>()
            strings.push(function_name.as_bytes().to_vec());
            w.write_action(&Action::Push(vec![Value::Str(SwfStr::from_bytes(
                strings.last().unwrap(),
            ))]))?;
            w.write_action(&Action::CallMethod)?;

            Self::dump_stack(&mut w)?;

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

            let (obj_name, func_name, arg_count_range) = self.select(static_methods);
            // Some functions take a variable argument counts, pick a random number of args to get good coverage
            // We ignore the lower bound here as we also want to test how missing args are handled in avm1
            // In avm2 we will want to make use of that, as missing args will cause exceptions
            let arg_count = self.rng.gen_range(0..=*arg_count_range.end());

            for _ in 0..arg_count {
                w.write_action(&Action::Push(vec![random_value(&mut self.rng, &mut strings)]))?;
            }

            w.write_action(&Action::Push(vec![
                Value::Int(arg_count),
                Value::Str(obj_name.into()),
            ]))?;
            w.write_action(&Action::GetVariable)?;
            w.write_action(&Action::Push(vec![Value::Str(func_name.into())]))?;
            w.write_action(&Action::CallMethod)?;

            Self::dump_stack(&mut w)?;
        }

        if OPCODE_FUZZ {
            self.opcode_fuzz(&mut w, &mut strings)?;
        }

        // Log a sentinal so we know that its done
        w.write_action(&Action::Push(vec![Value::Str("#CASE_COMPLETE#".into())]))?;
        w.write_action(&Action::Trace)?;

        w.write_action(&Action::GetUrl {
            target: "_root".into(),
            url: "fscommand:quit".into(),
        })?;

        // Create the swf
        swf::write_swf(
            &swf_header,
            &[
                Tag::DoAction(do_action_bytes.as_slice()),
                Tag::EnableDebugger(SwfStr::from_utf8_str("$1$5C$2dKTbwjNlJlNSvp9qvD651")),
            ],
            output_data,
        )?;

        Ok(())
    }
}
