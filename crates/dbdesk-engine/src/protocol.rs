/// A request from the UI to the engine.
///
/// There are no commands until M1 adds data sources. The channel exists already so the UI and
/// engine are wired once; closing it (by shutting the [`EngineHandle`](crate::EngineHandle)
/// down) is how the UI asks the engine to stop.
#[derive(Debug)]
pub enum Command {}

/// Something the engine reports to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The engine's command loop is running and accepting commands.
    Started,
}
