/// The three patterns `NSHapticFeedbackManager` offers; macOS has no custom waveform API, so
/// framework moments map onto these. A no-op on every other OS and on trackpads without Force
/// Touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Haptic {
    /// Something snapped into place: a guide, a detent, a zoom reaching 100%. Played when the
    /// frame showing the snap is drawn.
    Alignment,
    /// A discrete level crossed: a full-swipe threshold, a pressure stage, a stepper.
    LevelChange,
    /// Anything else worth feeling.
    Generic,
}
