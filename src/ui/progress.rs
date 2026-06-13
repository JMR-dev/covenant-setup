use super::*;

pub trait ProgressSink: Send {
    fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError>;
    fn log(&mut self, message: &str) -> Result<(), AppError>;
    fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError>;
    fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError>;
    fn cancel_requested(&mut self) -> bool {
        false
    }
    fn clear_cancel_request(&mut self) {}
}

impl ProgressSink for GuiProgress {
    fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError> {
        GuiProgress::advance(self, current_step, total_steps, message)
    }

    fn log(&mut self, message: &str) -> Result<(), AppError> {
        GuiProgress::log(self, message)
    }

    fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError> {
        GuiProgress::finish(self, message, wait_for_close)
    }

    fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError> {
        GuiProgress::fail(
            self,
            app_name,
            operation,
            message,
            error,
            errata,
            wait_for_close,
        )
    }

    fn cancel_requested(&mut self) -> bool {
        if !self.cancel_seen {
            self.cancel_seen = self.session.poll_cancel_request(&self.logger);
        }
        self.cancel_seen
    }

    fn clear_cancel_request(&mut self) {
        self.cancel_seen = false;
    }
}

pub struct GuiProgress {
    session: CSharpUiSession,
    logger: Logger,
    cancel_seen: bool,
}

impl GuiProgress {
    pub fn start(
        title: &str,
        app_name: Option<&str>,
        install_root: Option<&str>,
        branding_image: Option<&str>,
        show_welcome: bool,
        total_steps: usize,
        logger: &Logger,
    ) -> Result<Self, AppError> {
        let mut session = CSharpUiSession::start()?;

        session.send(&json!({
            "type": "init",
            "title": title,
            "app_name": app_name,
            "install_dir": install_root,
            "branding_image": branding_image,
            "total_steps": total_steps.max(1),
            "show_welcome": show_welcome,
        }))?;

        #[derive(serde::Deserialize)]
        struct WelcomeResponse {
            #[serde(rename = "type")]
            message_type: String,
            result: Option<String>,
        }

        // The explicit show_welcome flag is the single source of truth for
        // the handshake: the UI shows the welcome page and replies exactly
        // when the init message carries it, so the two sides cannot disagree
        // about whether a welcome_response is owed.
        if show_welcome {
            let response: WelcomeResponse = session.read()?;
            if response.message_type != "welcome_response"
                || response.result.as_deref() != Some("install")
            {
                return Err(AppError::CancelledByUser);
            }
        }

        Ok(Self {
            session,
            logger: logger.clone(),
            cancel_seen: false,
        })
    }

    pub fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "progress",
            "current_step": current_step,
            "total_steps": total_steps.max(1),
            "message": message,
        }))
    }

    pub fn log(&mut self, message: &str) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "log",
            "message": message,
        }))
    }

    pub fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "finish",
            "message": message,
        }))?;
        if wait_for_close {
            self.session.wait_for_exit()?;
        }
        Ok(())
    }

    pub fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "fail",
            "app_name": app_name,
            "operation": operation,
            "message": message,
            "error": error,
            "errata": errata,
        }))?;
        if wait_for_close {
            self.session.wait_for_exit()?;
        }
        Ok(())
    }
}
