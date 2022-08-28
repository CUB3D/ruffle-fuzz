//! Runner for running a fuzz case through Ruffle and extracting the output

use crate::MyError;
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
use std::time::{Duration, Instant};

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

pub async fn open_ruffle(bytes: &[u8]) -> Result<(String, Duration), MyError> {
    let ruffle_start = Instant::now();

    let movie = SwfMovie::from_data(&bytes, None, None).expect("Load movie fail");
    let log = StringLogger::default();

    let player = ruffle_core::PlayerBuilder::new()
        .with_renderer(NullRenderer::new(ViewportDimensions {
            height: 32,
            width: 32,
            scale_factor: 1.0,
        }))
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

        if Instant::now().duration_since(ruffle_start) > Duration::from_secs(30) {
            println!("Ruffle timed out, run > 30s");
            lock.set_is_playing(false);
        }
    }

    let lock = player.lock().unwrap();
    let out = lock.log_backend().__fuzz__get_log_string();
    Ok((out, Instant::now() - ruffle_start))
}
