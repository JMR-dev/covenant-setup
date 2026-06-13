use super::*;

pub(crate) fn get_locking_processes(path: &Path, logger: &Logger) -> Result<Vec<u32>, AppError> {
    let mut session = 0u32;
    let mut key = [0u16; 33];
    logger.unsafe_enter("RmStartSession", json!({}));
    let start_result = unsafe { RmStartSession(&mut session, Some(0), PWSTR(key.as_mut_ptr())) };
    logger.unsafe_exit(
        "RmStartSession",
        json!({"status":start_result.0,"session":session}),
    );
    win32_ok(start_result, "RmStartSession")?;

    let file = Utf16Arg::from_path(path);
    let resources = [PCWSTR(file.as_ptr())];
    logger.unsafe_enter("RmRegisterResources", json!({"path":path}));
    let register_result = unsafe { RmRegisterResources(session, Some(&resources), None, None) };
    logger.unsafe_exit("RmRegisterResources", json!({"status":register_result.0}));
    if let Err(err) = win32_ok(register_result, "RmRegisterResources") {
        let _ = end_restart_manager_session(session, logger);
        return Err(err);
    }

    let mut needed = 0u32;
    let mut count = 0u32;
    let mut reasons = 0u32;
    logger.unsafe_enter("RmGetList", json!({"phase":"probe"}));
    let probe = unsafe { RmGetList(session, &mut needed, &mut count, None, &mut reasons) };
    logger.unsafe_exit(
        "RmGetList",
        json!({"phase":"probe","status":probe.0,"needed":needed}),
    );
    if probe != ERROR_SUCCESS && probe != ERROR_MORE_DATA {
        let _ = end_restart_manager_session(session, logger);
        return win32_ok(probe, "RmGetList").map(|_| Vec::new());
    }
    if needed == 0 {
        end_restart_manager_session(session, logger)?;
        return Ok(Vec::new());
    }

    let mut processes = vec![RM_PROCESS_INFO::default(); needed as usize];
    count = needed;
    logger.unsafe_enter(
        "RmGetList",
        json!({"phase":"fetch","capacity":processes.len()}),
    );
    let fetch = unsafe {
        RmGetList(
            session,
            &mut needed,
            &mut count,
            Some(processes.as_mut_ptr()),
            &mut reasons,
        )
    };
    logger.unsafe_exit(
        "RmGetList",
        json!({"phase":"fetch","status":fetch.0,"count":count}),
    );
    end_restart_manager_session(session, logger)?;
    win32_ok(fetch, "RmGetList")?;
    if count as usize > processes.len() {
        return Err(AppError::Message(
            "Restart Manager count exceeded allocated buffer".into(),
        ));
    }
    Ok(processes
        .into_iter()
        .take(count as usize)
        .map(|p| p.Process.dwProcessId)
        .collect())
}

fn end_restart_manager_session(session: u32, logger: &Logger) -> Result<(), AppError> {
    logger.unsafe_enter("RmEndSession", json!({"session":session}));
    let result = unsafe { RmEndSession(session) };
    logger.unsafe_exit("RmEndSession", json!({"status":result.0}));
    win32_ok(result, "RmEndSession")
}
