use eframe::egui;
use std::fs;

pub(crate) struct Palette {
    pub(crate) background: egui::Color32,
    pub(crate) panel: egui::Color32,
    pub(crate) border: egui::Color32,
    pub(crate) text: egui::Color32,
    pub(crate) muted: egui::Color32,
    pub(crate) accent: egui::Color32,
    pub(crate) accent_dark: egui::Color32,
    pub(crate) accent_light: egui::Color32,
    pub(crate) mine: egui::Color32,
    pub(crate) danger: egui::Color32,
}

/// The color themes the select offers. The look (square corners, 1px borders, flat) is the same for all.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Choice {
    Dark,
    Light,
    Amber,
    Green,
}

impl Choice {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Choice::Dark => "Dark",
            Choice::Light => "Light",
            Choice::Amber => "Amber",
            Choice::Green => "Green",
        }
    }

    /// The egui base the palette is laid over: Light on light neutrals, the rest on the dark base.
    pub(crate) fn preference(self) -> egui::ThemePreference {
        match self {
            Choice::Light => egui::ThemePreference::Light,
            Choice::Dark | Choice::Amber | Choice::Green => egui::ThemePreference::Dark,
        }
    }
}

pub(crate) const fn rgb(hex: u32) -> egui::Color32 {
    egui::Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Dark: the colors of chromeExt-PicHunter (src/popup.css). Light: PicHunter has none, so this is the
/// same blue accent on light neutrals. Amber and Green: phosphor text on black.
pub(crate) fn palette(choice: Choice) -> Palette {
    match choice {
        Choice::Dark => Palette {
            background: rgb(0x1a1a2e), panel: rgb(0x16213e), border: rgb(0x2a3a5c), text: rgb(0xe0e0e0), muted: rgb(0xb0bec5),
            accent: rgb(0x4fc3f7), accent_dark: rgb(0x0288d1), accent_light: rgb(0x81d4fa), mine: rgb(0x81d4fa), danger: rgb(0xe57373),
        },
        Choice::Light => Palette {
            background: rgb(0xf5f7fa), panel: rgb(0xffffff), border: rgb(0xcfd8dc), text: rgb(0x1a1a2e), muted: rgb(0x546e7a),
            accent: rgb(0x0277bd), accent_dark: rgb(0x0288d1), accent_light: rgb(0x4fc3f7), mine: rgb(0x01579b), danger: rgb(0xc62828),
        },
        Choice::Amber => Palette {
            background: rgb(0x000000), panel: rgb(0x0d0900), border: rgb(0x7a5400), text: rgb(0xffb000), muted: rgb(0xb37a00),
            accent: rgb(0xffd24d), accent_dark: rgb(0x8a5f00), accent_light: rgb(0xffe08a), mine: rgb(0xffd24d), danger: rgb(0xff5533),
        },
        Choice::Green => Palette {
            background: rgb(0x050a08), panel: rgb(0x0a140f), border: rgb(0x417d55), text: rgb(0xb4dcbe), muted: rgb(0x526b59),
            accent: rgb(0x46ff82), accent_dark: rgb(0x417d55), accent_light: rgb(0x46ff82), mine: rgb(0x46ff82), danger: rgb(0xff5555),
        },
    }
}

/// The palette's colors on egui's dark or light base: square corners, 1px borders, no shadows, no hover growth.
pub(crate) fn visuals(palette: &Palette, theme: egui::Theme) -> egui::Visuals {
    let mut visuals = match theme {
        egui::Theme::Dark => egui::Visuals::dark(),
        egui::Theme::Light => egui::Visuals::light(),
    };
    let white = egui::Color32::WHITE;
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.background;
    visuals.extreme_bg_color = palette.panel;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent_dark;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, white);
    let rounding = egui::CornerRadius::ZERO;
    visuals.window_corner_radius = rounding;
    visuals.menu_corner_radius = rounding;
    visuals.window_shadow = egui::Shadow::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    for widget in [&mut visuals.widgets.noninteractive, &mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
        widget.corner_radius = rounding;
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border);
    visuals.widgets.inactive.weak_bg_fill = palette.panel;
    visuals.widgets.inactive.bg_fill = palette.panel;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, palette.muted);
    visuals.widgets.hovered.weak_bg_fill = palette.border;
    visuals.widgets.hovered.bg_fill = palette.border;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, palette.text);
    visuals.widgets.active.weak_bg_fill = palette.accent_dark;
    visuals.widgets.active.bg_fill = palette.accent_dark;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent_light);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, white);
    visuals
}

/// Galmuri11 (pixel font with Hangul, OFL, bundled in the exe) is the main font for all text.
/// Windows system fonts stay as fallbacks for any character Galmuri does not have.
pub(crate) fn install_korean_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("galmuri".to_owned(), egui::FontData::from_static(GALMURI).into());
    let fallbacks = ["C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\gulim.ttc"];
    if let Some(bytes) = fallbacks.iter().find_map(|path| fs::read(path).ok()) {
        fonts.font_data.insert("system".to_owned(), egui::FontData::from_owned(bytes).into());
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(0, "galmuri".to_owned());
        if fonts.font_data.contains_key("system") {
            list.push("system".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

pub(crate) static GALMURI: &[u8] = include_bytes!("../../assets/Galmuri11.ttf");

/// Panel frame that keeps text inside the bezel when the CRT look is on.
pub(crate) fn inset(palette: &Palette, crt: bool) -> egui::Frame {
    egui::Frame::new().fill(palette.background).inner_margin(egui::Margin::same(if crt { 12 } else { 6 }))
}
