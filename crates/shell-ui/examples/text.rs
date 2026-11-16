//! Text rendering showcase: `cargo run --example text`.

use std::error::Error;

use shell_ui::{
    AlignItems, Anchor, Color, Element, Extent, FontFamily, Layer, Margin, Ui, Window,
    calloop::EventLoop,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut event_loop = EventLoop::try_new()?;
    let mut ui = Ui::connect(&event_loop)?;

    let fg = Color::hex(0xcdd6f4);
    let pill = |label: &str, bg: u32| {
        Element::row()
            .padding_xy(10.0, 0.0)
            .align_items(AlignItems::CENTER)
            .fill_height()
            .background(Color::hex(bg))
            .corner_radius(11.0)
            .child(Element::text(label))
    };

    ui.open(
        Window::layer_shell("text-bar")
            .layer(Layer::Top)
            .anchor(Anchor::TOP)
            .width(Extent::Fill)
            .height(Extent::Fixed(32))
            .exclusive()
            .background(Color::hex(0x11111b))
            .root(
                Element::row()
                    .padding_xy(8.0, 5.0)
                    .gap(8.0)
                    .align_items(AlignItems::CENTER)
                    .font_size(13.0)
                    .text_color(fg)
                    .child(pill("1", 0x89b4fa).text_color(Color::hex(0x11111b)).bold())
                    .child(pill("2", 0x313244))
                    .child(Element::text("~/dev/shell — nvim").wrap(false))
                    .child(Element::spacer())
                    .child(Element::text("日本語 · العربية · Ελληνικά · 🦀🎉").wrap(false))
                    .child(Element::spacer())
                    .child(Element::text("12:34").bold().font_family(FontFamily::Monospace)),
            ),
    );

    ui.open(
        Window::layer_shell("text-panel")
            .layer(Layer::Overlay)
            .anchor(Anchor::TOP | Anchor::RIGHT)
            .margin(Margin::all(16))
            .width(Extent::Fixed(320))
            .height(Extent::Fit)
            .root(
                Element::column()
                    .padding(16.0)
                    .gap(10.0)
                    .background(Color::hex(0x1e1e2e).with_alpha(0.95))
                    .corner_radius(12.0)
                    .text_color(fg)
                    .child(Element::text("Notification").font_size(18.0).bold())
                    .child(Element::text(
                        "Body text wraps at word boundaries to the panel's width, and the \
                         window grows to fit however many lines that takes.",
                    ))
                    .child(Element::text("Italic, light weight").italic().font_weight(300))
                    .child(
                        Element::text("fn main() { println!(\"mono\"); }")
                            .font_family(FontFamily::Monospace),
                    )
                    .child(
                        Element::column()
                            .padding(10.0)
                            .background(Color::hex(0xf5e0dc))
                            .corner_radius(8.0)
                            .child(
                                Element::text("Dark text on a light background.")
                                    .text_color(Color::hex(0x1e1e2e)),
                            ),
                    ),
            ),
    );

    event_loop.run(None, &mut ui, |_| {})?;
    Ok(())
}
