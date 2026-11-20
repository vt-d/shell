//! The top bar.

use shell_m3e::ThemeExt;
use shell_ui::{AlignItems, Anchor, Element, Extent, Frame, JustifyContent, Layer, Window};

use crate::{
    clock::Clock, launcher::Launcher, media::Media, power::Power, quick::QuickSettings,
    status::Status, tray::Tray, workspaces::WorkspaceIndicator,
};

/// Bar height in logical pixels: room for the pills with 8px above and below. (Material's
/// docked toolbar is 64dp, sized for touch.)
pub const HEIGHT: u32 = 56;
/// Space between the bar's left and right edges and its content: Material's docked
/// toolbar leading and trailing space.
pub(crate) const MARGIN: f32 = 16.0;
/// How opaque the bar's background is: slightly see-through, so what's behind shows
/// through the compositor's blur (a Hyprland layer rule for the `bar` namespace).
pub const OPACITY: f32 = 0.8;
/// Space between the widgets on the right: Material's gap between button groups.
const GAP: f32 = 8.0;

/// What the bar shows; the event sources that feed it update this and refresh the bar.
pub struct Bar {
    pub workspaces: WorkspaceIndicator,
    pub media: Media,
    /// Before the status group, when there is anything in it.
    pub tray: Tray,
    pub status: Status,
    pub quick: QuickSettings,
    pub clock: Clock,
    pub power: Power,
    /// Not on the bar, but opened over it and closed along with what the bar opens.
    pub launcher: Launcher,
}

/// One of the bar's sides: as wide as the other, its content against `justify`.
fn side(justify: JustifyContent) -> Element {
    Element::row().width(0.0).grow(1.0).align_items(AlignItems::CENTER).justify_content(justify)
}

impl Bar {
    pub fn window(&mut self, frame: &mut Frame) -> Window {
        let theme = frame.theme();
        // The time is the last segment of the status group.
        let clock = self.clock.view(frame);
        // The tray leads the group, and brings its own seams, so that nothing is left of it
        // when it is empty.
        let tray = self.tray.view(frame);
        let status = self.status.view(frame, self.tray.presence(), clock);
        let status = Element::row().align_items(AlignItems::CENTER).child(tray).child(status);
        Window::layer_shell("bar")
            .layer(Layer::Top)
            .anchor(Anchor::TOP)
            .width(Extent::Fill)
            .height(Extent::Fixed(HEIGHT))
            .exclusive()
            .background(theme.surface.with_alpha(OPACITY))
            .root(
                // Three columns: the sides share what the middle leaves equally, so what's
                // playing is centered on the screen whatever is beside it. Workspaces
                // lead; status (ending in the time) and power trail.
                Element::row()
                    .align_items(AlignItems::CENTER)
                    .text_color(theme.on_surface)
                    .padding_xy(MARGIN, 0.0)
                    .child(side(JustifyContent::START).child(self.workspaces.view(frame)))
                    .child(self.media.pill(frame))
                    .child(
                        side(JustifyContent::END)
                            .gap(GAP)
                            .child(status)
                            .child(self.power.view(frame)),
                    ),
            )
    }
}
