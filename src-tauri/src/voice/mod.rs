mod capture;
mod wakeword;

use tauri::AppHandle;

#[derive(Default)]
pub struct VoiceState {
    handle: Option<capture::VoiceEngineHandle>,
}

impl VoiceState {
    pub fn start(&mut self, app: AppHandle) -> Result<(), String> {
        if self.handle.is_some() {
            return Ok(());
        }
        self.handle = Some(capture::spawn_listener(app)?);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }

    /// Suppresses the microphone pipeline while Aloe's own reply is being spoken aloud, so
    /// played-back TTS audio can't be picked up by the mic and misread as a new wake word
    /// or question.
    pub fn set_muted(&self, muted: bool) {
        if let Some(handle) = &self.handle {
            handle.set_muted(muted);
        }
    }

    pub fn resume_listening(&self) {
        if let Some(handle) = &self.handle {
            handle.resume_listening();
        }
    }

    pub fn stop_current_turn(&self) {
        if let Some(handle) = &self.handle {
            handle.stop_current_turn();
        }
    }
}
