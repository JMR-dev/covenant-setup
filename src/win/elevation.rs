use super::*;

pub(crate) fn quote_command_line_arg(arg: &str) -> String {
    let needs_quoting = arg.is_empty() || arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !needs_quoting {
        return arg.to_string();
    }

    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            }
            _ => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                out.push(c);
                backslashes = 0;
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
    out
}

pub fn is_elevated(logger: &Logger) -> Result<bool, AppError> {
    let mut token = HANDLE::default();
    logger.unsafe_enter("OpenProcessToken", json!({}));
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    logger.unsafe_exit("OpenProcessToken", json!({"opened": !token.is_invalid()}));

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    logger.unsafe_enter("GetTokenInformation", json!({"class":"TokenElevation"}));
    let info_result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    logger.unsafe_exit("GetTokenInformation", json!({"returned": returned}));
    close_handle(token, logger)?;
    info_result?;
    if returned < std::mem::size_of::<TOKEN_ELEVATION>() as u32 {
        return Err(AppError::Message("Short TOKEN_ELEVATION payload".into()));
    }
    Ok(elevation.TokenIsElevated != 0)
}

pub fn relaunch_as_admin(logger: &Logger) -> Result<(), AppError> {
    let exe = std::env::current_exe()?;
    let params = std::env::args()
        .skip(1)
        .map(|arg| quote_command_line_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    logger.unsafe_enter(
        "ShellExecuteW",
        json!({"verb":"runas","exe":exe,"params":params}),
    );
    let result = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            w!("runas"),
            PCWSTR(Utf16Arg::from_path(&exe).as_ptr()),
            PCWSTR(Utf16Arg::from_str(&params).as_ptr()),
            PCWSTR::null(),
            SW_SHOW,
        )
    };
    let code = result.0 as isize;
    logger.unsafe_exit("ShellExecuteW", json!({"hinstance": code}));
    if code <= 32 {
        return Err(AppError::Message(format!("ShellExecuteW failed: {code}")));
    }
    Ok(())
}
