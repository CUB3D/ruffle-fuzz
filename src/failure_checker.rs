use crate::ruffle_runner::open_ruffle;
use crate::FAILURES_DIR;
use std::error::Error;

pub async fn check_failures() -> Result<(), Box<dyn Error>> {
    let dir = std::fs::read_dir(FAILURES_DIR)?;

    let mut total = 0;
    let mut failed = 0;

    for entry in dir
        .flatten()
        .filter(|e| e.file_type().is_ok())
        .filter(|e| e.file_type().unwrap().is_dir())
    {
        let swf_path = entry.path().join("out.swf");
        let flash_output_path = entry.path().join("flash.txt");
        let swf_content = std::fs::read(swf_path)?;

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
