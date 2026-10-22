use dbdesk_engine::Event;

/// What the UI knows about the world, built only from engine events.
#[derive(Debug, Default)]
pub struct AppState {
    pub engine_status: EngineStatus,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EngineStatus {
    #[default]
    Starting,
    Running,
    Stopped,
}

impl EngineStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

impl AppState {
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Started => self.engine_status = EngineStatus::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_is_running_once_it_reports_started() {
        let mut state = AppState::default();
        assert_eq!(state.engine_status, EngineStatus::Starting);

        state.apply(Event::Started);

        assert_eq!(state.engine_status, EngineStatus::Running);
    }
}
