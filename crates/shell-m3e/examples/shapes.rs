//! Material 3 Expressive's shape library: `cargo run --example shapes`.
//!
//! Every shape, in the order of Material's spec, and below them a row continuously
//! morphing through the library.

use std::{error::Error, f32::consts::TAU, time::Instant};

use shell_m3e::{MaterialShape, ThemeExt};
use shell_ui::{
    Anchor, Element, Extent, Frame, Layer, Margin, Shape, Ui, Window, calloop::EventLoop,
};

const COLUMNS: usize = 7;
const CELL: f32 = 64.0;
const SHAPE: f32 = 48.0;

fn main() -> Result<(), Box<dyn Error>> {
    let mut event_loop = EventLoop::try_new()?;
    let mut ui = Ui::connect(&event_loop)?;
    let start = Instant::now();

    ui.open(move |frame: &mut Frame| {
        let theme = frame.theme();
        let cell = |shape: Shape| {
            Element::new()
                .size(CELL, CELL)
                .center()
                .child(Element::new().size(SHAPE, SHAPE).background(theme.primary).shape(shape))
        };

        let mut grid = Element::column();
        for row in MaterialShape::ALL.chunks(COLUMNS) {
            grid = grid.child(Element::row().children(row.iter().map(|&s| cell(s.into()))));
        }

        // Each shape in turn morphs into the next, then spins.
        let t = start.elapsed().as_secs_f32() / 1.5;
        frame.request_frame();
        let mut morphs = Element::row();
        for offset in 0..COLUMNS {
            let step = t.floor() as usize + offset * 5;
            let from = MaterialShape::ALL[step % MaterialShape::ALL.len()];
            let to = MaterialShape::ALL[(step + 1) % MaterialShape::ALL.len()];
            let progress = smoothstep((t.fract() * 2.0).min(1.0));
            morphs =
                morphs.child(cell(Shape::morph(from, to, progress).rotated(t.fract() * TAU / 8.0)));
        }

        Window::layer_shell("shapes")
            .layer(Layer::Overlay)
            .anchor(Anchor::TOP | Anchor::RIGHT)
            .margin(Margin::all(16))
            .width(Extent::Fit)
            .height(Extent::Fit)
            .root(
                Element::column()
                    .padding(12.0)
                    .gap(12.0)
                    .background(theme.surface_container)
                    .corner_radius(16.0)
                    .child(grid)
                    .child(morphs),
            )
    });

    event_loop.run(None, &mut ui, |_| {})?;
    Ok(())
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
