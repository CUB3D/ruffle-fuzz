use std::time::{Duration, Instant};

/// Use the linux `ptrace` API to inject swfs and hook log file writes, this allows running multiple flash instances in parallel
/// and improves perf by avoiding file system writes
pub async fn open_flash_ptrace(bytes: &[u8]) -> anyhow::Result<(String, Duration)> {
    let flash_start = Instant::now();

    let process_path = "./utils/flashplayer_32_sa_debug";
    let process_name = "flashplayer_32_sa_debug";
    let arg = "./test.swf";
    let mut ptrace = ptrace::Ptrace::new(process_path, process_name, arg).unwrap();
    ptrace.vfs_mut().mock_file(
        &["./test.swf", "/mnt/Media/torrent/flash-fuzz/./test.swf"],
        bytes.to_vec(),
    );
    ptrace.vfs_mut().mock_file(
        &["/home/cub3d/.macromedia/Flash_Player/Logs/flashlog.txt"],
        vec![0u8],
    );

    ptrace.spawn(Box::new(|_pt, event| {
        tracing::info!("Got event {:?}", event);
    }));

    let log_bytes = ptrace
        .vfs_mut()
        .get_file_content_by_path("/home/cub3d/.macromedia/Flash_Player/Logs/flashlog.txt")
        .unwrap();
    if log_bytes == [0] {
        panic!();
    }
    let log_content = String::from_utf8(log_bytes).unwrap();
    Ok((log_content, Instant::now() - flash_start))
}
