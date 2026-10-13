//! The only crate that names GPUI (ARCHITECTURE.md 2, 2.1). Everything else reaches it as
//! `nd_gpui::gpui`, and lays out with the taffy GPUI converts its styles into as `nd_gpui::taffy`.
//! The `fast` feature swaps both for gpui-fast and its taffy. The pinned snapshot, the fork branch
//! and the patch queue are described in `fork.toml`; `scripts/gpui-fork` applies, vendors, bumps
//! and gates them.

#[cfg(not(feature = "fast"))]
pub use gpui;
#[cfg(feature = "fast")]
pub use gpui_fast as gpui;

#[cfg(feature = "fast")]
pub use gpui_fast_platform as gpui_platform;
#[cfg(all(feature = "platform", not(feature = "fast")))]
pub use gpui_platform;

#[cfg(not(feature = "fast"))]
pub use taffy;
#[cfg(feature = "fast")]
pub use taffy_fast as taffy;

/// The two taffy versions behind one surface, for `nd-ui`'s layout of retained backends.
pub mod layout {
    use crate::taffy::{self, AvailableSpace, NodeId, Size, TaffyResult, TaffyTree};

    /// Taffy's type for `min_size` and `max_size`: `Dimension` in 0.13, `LengthPercentageAuto` in
    /// 0.14. Both have `length`, `percent` and `auto` constructors.
    #[cfg(not(feature = "fast"))]
    pub type MinMax = taffy::Dimension;
    #[cfg(feature = "fast")]
    pub type MinMax = taffy::LengthPercentageAuto;

    /// `compute_layout_with_measure` with one measure signature on both versions: known
    /// dimensions and available space in, the leaf's size out.
    pub fn compute_with_measure<C>(
        tree: &mut TaffyTree<C>,
        root: NodeId,
        available: Size<AvailableSpace>,
        mut measure: impl FnMut(Size<Option<f32>>, Size<AvailableSpace>, Option<&mut C>) -> Size<f32>,
    ) -> TaffyResult<()> {
        #[cfg(not(feature = "fast"))]
        return tree.compute_layout_with_measure(root, available, |known, space, _, context, _| {
            measure(known, space, context)
        });
        #[cfg(feature = "fast")]
        return tree.compute_layout_with_measure(root, available, |input, _, context, _| {
            taffy::LayoutOutput::from_outer_size(measure(input.known_dimensions, input.available_space, context))
        });
    }
}
