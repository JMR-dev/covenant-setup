use super::*;

/// Returns how many bytes are ready to read on `pipe` without blocking. Used
/// by the GUI progress session to poll for a `cancel_request` line: leaving a
/// blocking read pending on a synchronous pipe handle would serialize against
/// (and stall) progress writes on the same handle.
pub fn peek_named_pipe_available(pipe: &fs::File, logger: &Logger) -> Result<u32, AppError> {
    use std::os::windows::io::AsRawHandle;
    let mut available = 0u32;
    logger.unsafe_enter("PeekNamedPipe", json!({}));
    let result = unsafe {
        PeekNamedPipe(
            HANDLE(pipe.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut available),
            None,
        )
    };
    logger.unsafe_exit(
        "PeekNamedPipe",
        json!({"ok": result.is_ok(), "available": available}),
    );
    result?;
    Ok(available)
}
