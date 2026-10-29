//! Visual tokens and the widget kit.
//!
//! Colours live here, not in the screens: a screen asks [`palette`] for the
//! active theme and draws with those tokens. The same styles are installed for
//! both themes, so switching between light and dark is a preference change
//! rather than a second layout.
//!
//! The palette is deliberately small — background, panel, surface, border, text,
//! muted text, accent, and the three status colours. Anything else is a shade of
//! one of them.

// UI geometry is floating point; money never is, and never lives in this module.
#![allow(clippy::float_arithmetic)]

use egui::{
    Align, Color32, Context, CornerRadius, FontFamily, FontId, Frame, Margin, Painter, Pos2, Rect, Response, RichText,
    Sense, Shape, Stroke, StrokeKind, Style, TextStyle, Theme, ThemePreference, Ui, pos2, vec2,
};

/// Where the chosen appearance is kept between launches.
pub const STORAGE_KEY: &str = "appearance";

/// The colours one theme draws with.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The window behind everything.
    pub bg: Color32,
    /// The content panel.
    pub panel: Color32,
    /// The navigation rail.
    pub rail: Color32,
    /// A raised card or popup.
    pub surface: Color32,
    /// A quieter raised area, such as a table header or an open row.
    pub surface_alt: Color32,
    /// Hairlines and card edges.
    pub border: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary text and icons at rest.
    pub muted: Color32,
    /// The interactive colour.
    pub accent: Color32,
    /// Text on top of the accent.
    pub accent_text: Color32,
    /// A tinted version of the accent for selected rows and the rail.
    pub accent_weak: Color32,
    /// A confirmed result.
    pub success: Color32,
    /// Something worth reading.
    pub warning: Color32,
    /// Something that stopped or cannot be trusted.
    pub error: Color32,
    /// Every other row in a table.
    pub stripe: Color32,
    /// A row under the pointer.
    pub hover: Color32,
    /// A selected row.
    pub selection: Color32,
}

/// The light palette: cool grey paper with a blue accent.
pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xE7, 0xEA, 0xEF),
    panel: Color32::from_rgb(0xF6, 0xF7, 0xF9),
    rail: Color32::from_rgb(0xDE, 0xE2, 0xE8),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface_alt: Color32::from_rgb(0xEC, 0xF0, 0xF5),
    border: Color32::from_rgb(0xC2, 0xC8, 0xD1),
    text: Color32::from_rgb(0x15, 0x1A, 0x22),
    muted: Color32::from_rgb(0x46, 0x50, 0x5E),
    accent: Color32::from_rgb(0x1F, 0x5F, 0xD8),
    accent_text: Color32::WHITE,
    accent_weak: Color32::from_rgb(0xD9, 0xE5, 0xFA),
    success: Color32::from_rgb(0x16, 0x6B, 0x45),
    warning: Color32::from_rgb(0x7A, 0x52, 0x00),
    error: Color32::from_rgb(0xA1, 0x1B, 0x12),
    stripe: Color32::from_rgb(0xEF, 0xF2, 0xF6),
    hover: Color32::from_rgb(0xDE, 0xE5, 0xF0),
    selection: Color32::from_rgb(0xCF, 0xE0, 0xFA),
};

/// The dark palette: charcoal surfaces with a brighter blue accent.
pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x14, 0x16, 0x1B),
    panel: Color32::from_rgb(0x1C, 0x1F, 0x25),
    rail: Color32::from_rgb(0x16, 0x18, 0x1D),
    surface: Color32::from_rgb(0x23, 0x27, 0x2F),
    surface_alt: Color32::from_rgb(0x27, 0x2C, 0x35),
    border: Color32::from_rgb(0x35, 0x3B, 0x46),
    text: Color32::from_rgb(0xE7, 0xEA, 0xF0),
    muted: Color32::from_rgb(0x9A, 0xA3, 0xB2),
    accent: Color32::from_rgb(0x5B, 0x97, 0xFF),
    accent_text: Color32::from_rgb(0x0C, 0x10, 0x16),
    accent_weak: Color32::from_rgb(0x25, 0x38, 0x5A),
    success: Color32::from_rgb(0x4A, 0xD6, 0x8C),
    warning: Color32::from_rgb(0xE8, 0xB3, 0x4B),
    error: Color32::from_rgb(0xF9, 0x72, 0x6B),
    stripe: Color32::from_rgb(0x20, 0x24, 0x2B),
    hover: Color32::from_rgb(0x2B, 0x31, 0x3B),
    selection: Color32::from_rgb(0x2B, 0x3F, 0x63),
};

/// The tokens for the theme currently in use.
pub fn palette(ctx: &Context) -> &'static Palette {
    match ctx.theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// The tokens for a specific theme.
pub const fn palette_of(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// Parses the stored appearance name.
pub fn from_name(name: &str) -> Option<ThemePreference> {
    match name {
        "system" => Some(ThemePreference::System),
        "light" => Some(ThemePreference::Light),
        "dark" => Some(ThemePreference::Dark),
        _ => None,
    }
}

/// Installs both themes' styles. The preference chooses which one is active.
pub fn install(ctx: &Context) {
    for theme in [Theme::Light, Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
}

/// Switches the active theme and asks for a repaint.
pub fn set(ctx: &Context, preference: ThemePreference) {
    ctx.set_theme(preference);
    ctx.request_repaint();
}

fn style(theme: Theme) -> Style {
    let tokens = palette_of(theme);
    let mut visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };

    let radius = CornerRadius::same(6);
    visuals.panel_fill = tokens.panel;
    visuals.window_fill = tokens.panel;
    visuals.extreme_bg_color = tokens.bg;
    visuals.faint_bg_color = tokens.stripe;
    visuals.code_bg_color = tokens.surface_alt;
    visuals.override_text_color = None;
    visuals.warn_fg_color = tokens.warning;
    visuals.error_fg_color = tokens.error;
    visuals.hyperlink_color = tokens.accent;
    visuals.selection.bg_fill = tokens.selection;
    visuals.selection.stroke = Stroke::new(1.0, tokens.accent);
    visuals.window_stroke = Stroke::new(1.0, tokens.border);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = radius;
    visuals.weak_text_color = Some(tokens.muted);
    visuals.weak_text_alpha = 1.0;

    visuals.widgets.noninteractive = widget(tokens.panel, tokens.border, tokens.text, radius);
    visuals.widgets.inactive = widget(tokens.surface_alt, tokens.border, tokens.text, radius);
    visuals.widgets.hovered = widget(tokens.hover, tokens.accent, tokens.text, radius);
    visuals.widgets.active = widget(tokens.accent_weak, tokens.accent, tokens.text, radius);
    visuals.widgets.open = widget(tokens.surface_alt, tokens.border, tokens.text, radius);

    let mut style = Style {
        visuals,
        ..Default::default()
    };
    style.spacing.item_spacing = vec2(10.0, 8.0);
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.interact_size = vec2(48.0, 28.0);
    style.spacing.window_margin = Margin::same(16);
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.indent = 18.0;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.bar_inner_margin = 4.0;
    // The user reads the archive; selecting a label is never what a click meant.
    style.interaction.selectable_labels = false;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into_iter()
    .collect();
    style
}

fn widget(fill: Color32, border: Color32, text: Color32, radius: CornerRadius) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, border),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// A raised card with a title.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let tokens = palette(ui.ctx());
    Frame::new()
        .fill(tokens.surface)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(8)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().size(15.0));
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}

/// The icons the rail speaks in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    /// Find documents.
    Search,
    /// Start an invoice.
    Create,
    /// The party list.
    Customers,
    /// Archive settings.
    Settings,
    /// Leave the archive locked.
    Lock,
}

/// A square icon button for the navigation rail.
///
/// The icon is painted rather than typed, so it does not depend on a font that
/// happens to contain the glyph. The name is still the tooltip.
pub fn rail_button(ui: &mut Ui, icon: Icon, selected: bool, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
    let tokens = palette(ui.ctx());
    let painter = ui.painter();

    if selected || response.hovered() {
        let fill = if selected { tokens.accent_weak } else { tokens.hover };
        painter.rect_filled(rect, 9.0, fill);
    }
    if selected {
        let bar = Rect::from_min_size(
            pos2(rect.left() + 3.0, rect.top() + 11.0),
            vec2(3.0, rect.height() - 22.0),
        );
        painter.rect_filled(bar, 2.0, tokens.accent);
    }

    let colour = if selected {
        tokens.accent
    } else if response.hovered() {
        tokens.text
    } else {
        tokens.muted
    };
    draw_icon(painter, rect.shrink(12.0), icon, colour);
    response.on_hover_text(tooltip)
}

/// A small triangle marking the sorted column.
pub fn sort_caret(painter: &Painter, centre: Pos2, ascending: bool, colour: Color32) {
    let half = 4.0;
    let (tip, left, right) = if ascending {
        (
            pos2(centre.x, centre.y - half),
            pos2(centre.x - half, centre.y + half),
            pos2(centre.x + half, centre.y + half),
        )
    } else {
        (
            pos2(centre.x, centre.y + half),
            pos2(centre.x - half, centre.y - half),
            pos2(centre.x + half, centre.y - half),
        )
    };
    painter.add(Shape::convex_polygon(vec![tip, left, right], colour, Stroke::NONE));
}

fn draw_icon(painter: &Painter, rect: Rect, icon: Icon, colour: Color32) {
    let stroke = Stroke::new(1.6, colour);
    let centre = rect.center();
    let radius = rect.width().min(rect.height()) * 0.5;

    match icon {
        Icon::Search => {
            let lens = pos2(centre.x - radius * 0.12, centre.y - radius * 0.12);
            painter.circle_stroke(lens, radius * 0.52, stroke);
            painter.line_segment(
                [
                    pos2(lens.x + radius * 0.37, lens.y + radius * 0.37),
                    pos2(centre.x + radius * 0.72, centre.y + radius * 0.72),
                ],
                Stroke::new(2.0, colour),
            );
        }
        Icon::Create => {
            painter.line_segment(
                [
                    pos2(centre.x - radius * 0.55, centre.y),
                    pos2(centre.x + radius * 0.55, centre.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(centre.x, centre.y - radius * 0.55),
                    pos2(centre.x, centre.y + radius * 0.55),
                ],
                stroke,
            );
        }
        Icon::Customers => {
            painter.circle_stroke(pos2(centre.x, centre.y - radius * 0.32), radius * 0.28, stroke);
            painter.add(Shape::line(
                vec![
                    pos2(centre.x - radius * 0.62, centre.y + radius * 0.65),
                    pos2(centre.x - radius * 0.30, centre.y + radius * 0.18),
                    pos2(centre.x, centre.y + radius * 0.12),
                    pos2(centre.x + radius * 0.30, centre.y + radius * 0.18),
                    pos2(centre.x + radius * 0.62, centre.y + radius * 0.65),
                ],
                stroke,
            ));
        }
        Icon::Settings => {
            for (offset, knob) in [(-0.45_f32, 0.20_f32), (0.0, -0.28), (0.45, 0.34)] {
                let y = centre.y + offset * radius;
                painter.line_segment(
                    [pos2(centre.x - radius * 0.62, y), pos2(centre.x + radius * 0.62, y)],
                    stroke,
                );
                painter.circle_filled(pos2(centre.x + knob * radius, y), radius * 0.15, colour);
            }
        }
        Icon::Lock => {
            let body = Rect::from_center_size(
                pos2(centre.x, centre.y + radius * 0.30),
                vec2(radius * 1.02, radius * 0.76),
            );
            painter.rect_stroke(body, 3.0, stroke, StrokeKind::Middle);
            let shackle = body.center_top();
            let points: Vec<Pos2> = (0..=12)
                .map(|step| {
                    let angle = core::f32::consts::PI + (step as f32 / 12.0) * core::f32::consts::PI;
                    pos2(
                        shackle.x + radius * 0.42 * angle.cos(),
                        shackle.y + radius * 0.42 * angle.sin(),
                    )
                })
                .collect();
            painter.add(Shape::line(points, stroke));
        }
    }
}

/// Truncates a single line of text to the width of its rectangle.
pub fn cell_label(ui: &mut Ui, rect: Rect, value: &str, align: Align, strong: bool, colour: Color32) -> Response {
    let layout = match align {
        Align::Max => egui::Layout::right_to_left(Align::Center),
        _ => egui::Layout::left_to_right(Align::Center),
    };
    let mut rich = RichText::new(value).color(colour);
    if strong {
        rich = rich.strong();
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.add(egui::Label::new(rich).truncate().selectable(false))
    })
    .inner
}

/// A read-only value: prose around it is not selectable, but an amount, a name,
/// a hash, or a reference is something the user may want to copy.
pub fn value(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Label::new(RichText::new(text).color(palette(ui.ctx()).text)).selectable(true))
}

/// The same, in the monospaced face, for identifiers and digests.
pub fn value_mono(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Label::new(RichText::new(text).monospace().color(palette(ui.ctx()).text)).selectable(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_themes_are_readable_at_a_glance() {
        for palette in [&LIGHT, &DARK] {
            assert_ne!(palette.text, palette.bg);
            assert_ne!(palette.text, palette.panel);
            assert_ne!(palette.muted, palette.panel);
            assert_ne!(palette.accent, palette.panel);
        }
        assert!(LIGHT.text.r() < LIGHT.bg.r(), "light text is dark");
        assert!(DARK.text.r() > DARK.bg.r(), "dark text is light");
    }

    #[test]
    fn text_holds_contrast_on_every_surface() {
        for palette in [&LIGHT, &DARK] {
            for (name, background) in [
                ("panel", palette.panel),
                ("surface", palette.surface),
                ("surface_alt", palette.surface_alt),
                ("rail", palette.rail),
                ("selection", palette.selection),
            ] {
                let primary = contrast(palette.text, background);
                assert!(primary >= 4.5, "text on {name}: {primary:.2}");
                let secondary = contrast(palette.muted, background);
                assert!(secondary >= 3.0, "muted text on {name}: {secondary:.2}");
            }
            let button = contrast(palette.text, palette.surface_alt);
            assert!(button >= 4.5, "button text: {button:.2}");
            let accent = contrast(palette.accent_text, palette.accent);
            assert!(accent >= 4.5, "text on the accent: {accent:.2}");
        }
    }

    fn contrast(left: Color32, right: Color32) -> f32 {
        let left = luminance(left);
        let right = luminance(right);
        let (light, dark) = if left > right { (left, right) } else { (right, left) };
        (light + 0.05) / (dark + 0.05)
    }

    fn luminance(colour: Color32) -> f32 {
        fn channel(value: u8) -> f32 {
            let value = f32::from(value) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
    }

    #[test]
    fn the_stored_name_round_trips() {
        for preference in [ThemePreference::System, ThemePreference::Light, ThemePreference::Dark] {
            assert_eq!(from_name(preference.name()), Some(preference));
        }
        assert_eq!(from_name("sepia"), None);
    }

    #[test]
    fn installing_turns_off_selectable_labels() {
        let context = Context::default();
        install(&context);
        for theme in [Theme::Light, Theme::Dark] {
            assert!(!context.style_of(theme).interaction.selectable_labels);
        }
    }

    #[test]
    fn a_theme_switch_changes_the_active_palette() {
        let context = Context::default();
        install(&context);
        set(&context, ThemePreference::Light);
        assert_eq!(context.theme(), Theme::Light);
        set(&context, ThemePreference::Dark);
        assert_eq!(context.theme(), Theme::Dark);
    }
}
