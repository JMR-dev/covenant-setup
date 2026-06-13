use super::*;

pub fn report_error(message: &str) -> Result<(), AppError> {
    crate::trace_event("ui_prompt_report_error", json!({"message": message}));
    let _ = prompt(
        "covenant-setup",
        message,
        PromptButtons::Ok,
        PromptIcon::Error,
    )?;
    Ok(())
}

pub fn report_uninstall_success(app_name: &str) -> Result<(), AppError> {
    crate::trace_event(
        "ui_prompt_report_uninstall_success",
        json!({"app_name": app_name}),
    );
    let _ = prompt(
        "covenant-setup",
        &format!("{app_name} uninstalled successfully!"),
        PromptButtons::Ok,
        PromptIcon::Information,
    )?;
    Ok(())
}

pub fn prompt_uninstall_reboot(app_name: &str) -> Result<bool, AppError> {
    crate::trace_event("ui_prompt_uninstall_reboot", json!({"app_name": app_name}));
    let result = prompt(
        "covenant-setup",
        &format!(
            "{app_name} uninstalled successfully! Some files from the program still remain on your computer. To complete removal of these files, restart your computer now."
        ),
        PromptButtons::YesNo,
        PromptIcon::Information,
    )?;
    Ok(matches!(result, PromptResult::Yes))
}

pub(crate) fn prompt(
    title: &str,
    message: &str,
    buttons: PromptButtons,
    icon: PromptIcon,
) -> Result<PromptResult, AppError> {
    let mut session = CSharpUiSession::start()?;
    let id = format!("prompt-{}", unique_suffix());
    session.send(&json!({
        "type": "prompt",
        "id": id,
        "title": title,
        "message": message,
        "buttons": buttons.as_str(),
        "icon": icon.as_str(),
    }))?;
    let response: PromptResponse = session.read()?;
    if response.message_type != "prompt_response" || response.id.as_deref() != Some(&id) {
        return Err(AppError::Message("Unexpected UI prompt response".into()));
    }
    let result = PromptResult::from_str(response.result.as_deref().unwrap_or("none"))?;
    crate::trace_event(
        "ui_prompt_response",
        json!({"id": id, "result": response.result}),
    );
    Ok(result)
}

pub(crate) enum PromptButtons {
    Ok,
    YesNo,
}

impl PromptButtons {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::YesNo => "yes_no",
        }
    }
}

pub(crate) enum PromptIcon {
    Information,
    Error,
}

impl PromptIcon {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Information => "information",
            Self::Error => "error",
        }
    }
}

pub(crate) enum PromptResult {
    Ok,
    Cancel,
    Yes,
    No,
    None,
}

impl PromptResult {
    pub(crate) fn from_str(value: &str) -> Result<Self, AppError> {
        match value {
            "ok" => Ok(Self::Ok),
            "cancel" => Ok(Self::Cancel),
            "yes" => Ok(Self::Yes),
            "no" => Ok(Self::No),
            "none" => Ok(Self::None),
            other => Err(AppError::Message(format!(
                "Unknown UI prompt response: {other}"
            ))),
        }
    }
}
#[derive(serde::Deserialize)]
pub(crate) struct PromptResponse {
    #[serde(rename = "type")]
    message_type: String,
    id: Option<String>,
    result: Option<String>,
}
