//! The egui screens of the window: the chat and intro screens, the top bar, peer panel, verify and help windows,
//! the CRT frame, and the connection settings window (relay_form).

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) mod theme;
pub(crate) mod window;
pub(crate) mod relay_form;
pub(crate) use theme::*;
pub(crate) use window::*;
use crate::core::*;

pub(crate) const INPUT_HEIGHT: f32 = 36.0;
pub(crate) const TOAST_SECONDS: f32 = 3.0;
/// Which screen is shown. The connection work runs in the background on both.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Screen {
    Intro,
    Chat,
}

pub(crate) struct App {
    pub(crate) screen: Screen,
    pub(crate) shared: Arc<Mutex<Shared>>,
    pub(crate) outgoing: Sender<String>,
    pub(crate) input: String,
    pub(crate) theme: Choice,
    /// Scanline overlay that imitates an old CRT monitor. On by default; the top bar checkbox turns it off.
    pub(crate) crt: bool,
    pub(crate) hwnd: isize,
    /// This PC's address on the network, shown with the own entry in the peer panel.
    pub(crate) local_ip: String,
    /// The theme the title bar was last colored for.
    pub(crate) frame_theme: Option<Choice>,
    /// Until when the phone address and token are on screen. Only this window shows them: never a chat line, never a log.
    pub(crate) phone_until: Option<Instant>,
    /// The auto-show at the first chat screen was decided (it happens once, and only when this window's node is the host).
    pub(crate) phone_checked: bool,
    /// Set when the settings were saved: the session asks the serve node to start the relay again.
    pub(crate) reload: Arc<AtomicBool>,
    /// The Ctrl+R settings window, while it is open.
    pub(crate) relay_form: Option<relay_form::Form>,
    /// No connection mode saved yet: the first-run choice comes before the chat.
    pub(crate) first_run: bool,
    /// The last result line (saved, or left without saving), shown in the top bar.
    pub(crate) notice: String,
    /// The 30-second default of the first-run choice. None until the choice is shown.
    pub(crate) choice_timer: Option<talk_core::choice_timer::ChoiceTimer>,
    /// The mode picked on the first screen (Local or Supabase); 입장 enters with it.
    pub(crate) intro_mode: talk_core::relay_settings::Mode,
    /// The toast on screen: the text last shown and when it appeared (it fades after TOAST_SECONDS).
    pub(crate) toast_text: String,
    pub(crate) toast_started: Option<Instant>,
    /// The Ctrl+H / F1 help window, while it is open.
    pub(crate) help_open: bool,
    /// The verify window for a waiting peer, while it is open: (name, code).
    pub(crate) verify: Option<(String, String)>,
}

/// The key list of the TUI /help box, in its order (talk-core locale keys; "" is a gap).
pub(crate) const HELP_KEYS: &[&str] = &[
    "help.title_send", "help.send", "", "help.commands", "help.name", "help.lang", "help.codes",
    "help.peers", "help.trust", "help.phone", "help.help", "help.quit", "", "help.keys", "help.enter", "help.backspace",
    "help.ctrl_l", "help.ctrl_p", "help.ctrl_c",
];

pub(crate) const PHONE_SHOWN: Duration = Duration::from_secs(30);

/// F3 pressed this frame: switch the language (saved like the TUI's F3) and return the notice, if any. `None`: F3 was not pressed.
pub(crate) fn language_key(ctx: &egui::Context) -> Option<String> {
    ctx.input(|input| input.key_pressed(egui::Key::F3)).then(talk_core::i18n::switch_language_notice)
}

impl App {
    /// One frame of the settings window: Stay, Saved (reload the relay; first run goes to the chat), or Discard.
    pub(crate) fn settings_frame(&mut self, ctx: &egui::Context, palette: &Palette) {
        let Some(form) = self.relay_form.as_mut() else { return };
        let action = relay_form::show(ctx, palette, form);
        let message = form.message.clone();
        match action {
            relay_form::Action::Stay => {}
            relay_form::Action::Saved => {
                self.reload.store(true, Ordering::SeqCst);
                self.notice = message;
                self.relay_form = None;
                if self.first_run || self.screen == Screen::Intro {
                    self.first_run = false;
                    self.screen = Screen::Chat;
                }
            }
            relay_form::Action::Discard => {
                self.relay_form = None;
                // Back to the first-run choice: its 30 seconds start again.
                self.choice_timer = None;
                self.notice = talk_core::t("settings.cancel_discard", &[]);
            }
        }
        if self.crt {
            draw_scanlines(ctx);
            draw_bezel(ctx);
        }
    }

    /// First screen: name, one line about the app, the connection state, and the button into the chat.
    pub(crate) fn intro(&mut self, ctx: &egui::Context, palette: &Palette, _status: &str) {
        let mode = self.intro_mode;
        let supabase = mode == talk_core::relay_settings::Mode::Supabase;
        let picked_label = if supabase { talk_core::t("gui.mode.supabase", &[]) } else { talk_core::t("gui.mode.local", &[]) };
        let mut enter = false;
        let mut select = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.10);
                ui.colored_label(palette.accent, egui::RichText::new("herdr-talk").size(36.0));
                ui.add_space(8.0);
                ui.label(talk_core::t("gui.intro", &[]));
                ui.add_space(14.0);
                // The selected mode, then the selector: [로컬/자체] [Supabase].
                ui.colored_label(palette.accent, egui::RichText::new(picked_label).size(18.0));
                ui.add_space(8.0);
                // Two equal buttons, centered by padding the row on the left (the row width is 2 * 160 + spacing 8).
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - 328.0) / 2.0).max(0.0));
                    let size = egui::vec2(160.0, 36.0);
                    if ui.add(egui::Button::new(egui::RichText::new(talk_core::t("gui.mode.local_button", &[])).size(16.0)).selected(!supabase).min_size(size)).clicked() {
                        select = Some(talk_core::relay_settings::Mode::Local);
                    }
                    if ui.add(egui::Button::new(egui::RichText::new("Supabase").size(16.0)).selected(supabase).min_size(size)).clicked() {
                        select = Some(talk_core::relay_settings::Mode::Supabase);
                    }
                });
                ui.add_space(14.0);
                if ui.add(egui::Button::new(egui::RichText::new(talk_core::t("gui.button.enter", &[])).size(18.0)).min_size(egui::vec2(160.0, 40.0))).clicked() {
                    enter = true;
                }
            });
        });
        if let Some(picked) = select {
            self.intro_mode = picked;
        }
        if enter {
            self.enter(self.intro_mode);
        }
    }

    /// Enter the chat in `mode`: save the mode, let the relay reload. Supabase with missing settings opens the form first.
    pub(crate) fn enter(&mut self, mode: talk_core::relay_settings::Mode) {
        let state = state_dir();
        if mode == talk_core::relay_settings::Mode::Local {
            match talk_core::relay_settings::set_mode(&state, mode) {
                Ok(()) => {
                    self.reload.store(true, Ordering::SeqCst);
                    self.screen = Screen::Chat;
                }
                Err(error) => self.notice = error,
            }
            return;
        }
        let saved = talk_core::relay_settings::load_saved(&state);
        let as_supabase = talk_core::relay_settings::Settings { mode, ..saved };
        if talk_core::relay_settings::validate(&as_supabase).is_ok() {
            match talk_core::relay_settings::set_mode(&state, mode) {
                Ok(()) => {
                    self.reload.store(true, Ordering::SeqCst);
                    self.screen = Screen::Chat;
                }
                Err(error) => self.notice = error,
            }
        } else {
            // Missing settings: the form opens, and a successful save goes straight into the chat.
            self.relay_form = Some(relay_form::Form::open_supabase(&state));
        }
    }
}

impl App {
    /// A short message at the bottom of the window. A new message restarts the timer; it fades after TOAST_SECONDS.
    pub(crate) fn show_toast(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.notice != self.toast_text {
            self.toast_text = self.notice.clone();
            self.toast_started = (!self.notice.is_empty()).then(Instant::now);
        }
        let Some(started) = self.toast_started else { return };
        let left = TOAST_SECONDS - started.elapsed().as_secs_f32();
        if left <= 0.0 {
            self.toast_started = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        let alpha = if left < 0.5 { left / 0.5 } else { 1.0 };
        let text = self.toast_text.clone();
        egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -64.0)).interactable(false).show(ctx, |ui| {
            egui::Frame::NONE
                .fill(palette.panel.gamma_multiply(alpha))
                .stroke(egui::Stroke::new(1.0_f32, palette.accent.gamma_multiply(alpha)))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.colored_label(palette.text.gamma_multiply(alpha), text);
                });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(POLL);
        // eframe may re-apply the OS theme's visuals, so the chosen theme's colors are set every frame.
        let palette = palette(self.theme);
        // F3 changes the language on every screen; the texts drawn below already use it.
        if let Some(notice) = language_key(ctx) {
            if !notice.is_empty() {
                self.notice = notice;
            }
        }
        self.show_toast(ctx, &palette);
        ctx.set_visuals(visuals(&palette, ctx.theme()));
        if self.frame_theme != Some(self.theme) {
            style_frame(self.hwnd, &palette);
            self.frame_theme = Some(self.theme);
        }
        // The settings window takes the whole window while it is open (first run, or the top-bar button).
        if self.relay_form.is_some() {
            self.settings_frame(ctx, &palette);
            return;
        }
        // First run: pick the connection mode before anything else. Local is saved at once; Supabase opens the settings.
        if self.first_run {
            // Any input stops the 30-second default; when it runs out, the chat starts in Local mode and nothing is saved.
            let input_seen = ctx.input(|input| {
                input.events.iter().any(|event| {
                    matches!(event, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_) | egui::Event::PointerButton { pressed: true, .. })
                })
            });
            if relay_form::tick(&mut self.choice_timer, Instant::now(), input_seen) == relay_form::Tick::Start {
                self.first_run = false;
                self.choice_timer = None;
                self.screen = Screen::Chat;
            }
        }
        if self.first_run {
            let remaining = self.choice_timer.and_then(|timer| timer.remaining(Instant::now()));
            match relay_form::choose(ctx, &palette, remaining) {
                Some(relay_form::Pick::Local) => {
                    match talk_core::relay_settings::set_mode(&state_dir(), talk_core::relay_settings::Mode::Local) {
                        Ok(()) => {
                            self.reload.store(true, Ordering::SeqCst);
                            self.first_run = false;
                            self.choice_timer = None;
                            self.screen = Screen::Chat;
                        }
                        Err(error) => self.notice = error,
                    }
                }
                Some(relay_form::Pick::Supabase) => {
                    self.choice_timer = None;
                    self.relay_form = Some(relay_form::Form::open_supabase(&state_dir()));
                }
                None => {}
            }
            if self.first_run {
                if self.crt {
                    draw_scanlines(ctx);
                    draw_bezel(ctx);
                }
                return;
            }
        }
        if self.screen == Screen::Intro {
            let status = locked(&*self.shared, "update.intro").status.clone();
            self.intro(ctx, &palette, &status);
            return;
        }
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::R)) {
            self.relay_form = Some(relay_form::Form::open(&state_dir()));
            return;
        }
        if ctx.input(|input| (input.modifiers.ctrl && input.key_pressed(egui::Key::H)) || input.key_pressed(egui::Key::F1)) {
            self.help_open = true;
        }
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::L)) {
            locked(&*self.shared, "update.clear").clear = true;
        }
        let state = locked(&*self.shared, "update.chat");
        // Once, at the first chat screen: show the phone address when the node this window talks to is the host (방장) of this PC.
        if !self.phone_checked {
            if let Some((_, port)) = &state.me {
                self.phone_checked = true;
                if talk_core::phone::read(&state_dir()).is_some_and(|phone| phone.owner_port == *port) {
                    self.phone_until = Some(Instant::now() + PHONE_SHOWN);
                }
            }
        }
        // Ctrl+P toggles; typing or Enter hides.
        let (toggle, typed) = ctx.input(|input| (
            input.modifiers.ctrl && input.key_pressed(egui::Key::P),
            input.events.iter().any(|event| matches!(event, egui::Event::Text(_))) || input.key_pressed(egui::Key::Enter),
        ));
        let visible = self.phone_until.is_some_and(|until| until > Instant::now());
        if toggle {
            self.phone_until = if visible { None } else { Some(Instant::now() + PHONE_SHOWN) };
        } else if typed || !visible {
            self.phone_until = None;
        }
        let mut open_settings = false;
        let supabase = talk_core::relay_settings::Mode::from_saved(&state.mode) == talk_core::relay_settings::Mode::Supabase;
        let hosting = talk_core::phone::read(&state_dir()).is_some_and(|phone| state.me.as_ref().is_some_and(|(_, port)| phone.owner_port == *port));
        let (status_short, status_kind) = status_label(&state.status, supabase);
        egui::TopBottomPanel::top("status")
            .exact_height(40.0)
            .frame(egui::Frame::new().fill(palette.background).stroke(egui::Stroke::new(1.0_f32, palette.border)).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let color = match status_kind {
                    Kind::Ok => palette.accent,
                    Kind::Pending => palette.muted,
                    Kind::Problem => palette.danger,
                };
                ui.colored_label(palette.accent, egui::RichText::new("▣ HERDR-TALK").monospace().strong());
                if hosting {
                    ui.colored_label(palette.accent, talk_core::t("host.badge", &[]));
                }
                let shown = if state.status.starts_with(&status_connected()) {
                    talk_core::t("gui.status.mode_connected", &[("mode", &mode_name(supabase))])
                } else if state.status.starts_with(&status_embedded()) {
                    talk_core::t("gui.status.mode_connected", &[("mode", &mode_name(false))])
                } else {
                    status_short
                };
                ui.colored_label(color, egui::RichText::new(shown).monospace());
                if ui.button(talk_core::t("gui.button.connection_mode", &[])).clicked() {
                    open_settings = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(self.theme.name())
                        .show_ui(ui, |ui| {
                            for choice in [Choice::Dark, Choice::Light, Choice::Amber, Choice::Green] {
                                ui.selectable_value(&mut self.theme, choice, choice.name());
                            }
                        });
                    ui.checkbox(&mut self.crt, "CRT");
                        ui.colored_label(palette.muted, egui::RichText::new("SYS.VER 1.0").monospace());
                });
            });
        });
        ctx.set_theme(self.theme.preference());
        if open_settings {
            self.relay_form = Some(relay_form::Form::open(&state_dir()));
            ctx.request_repaint();
        }
        let mut verify_open = None;
        egui::SidePanel::right("peers").default_width(225.0).frame(inset(&palette, self.crt)).show(ctx, |ui| {
            ui.colored_label(palette.accent, egui::RichText::new(talk_core::t("gui.nodes.title", &[])).monospace().heading());
            if let Some((name, port)) = &state.me {
                ui.colored_label(palette.accent, egui::RichText::new(format!("● HOST / YOU  {name}")).monospace());
                ui.colored_label(palette.muted, egui::RichText::new(format!("{}:{port}", self.local_ip)).monospace());
                ui.separator();
            }
            // Supabase mode lists the people in the room by name; the LAN mode lists the peers found on the network.
            let shown = if supabase { &state.room_names } else { &state.peers };
            if shown.is_empty() {
                ui.colored_label(palette.muted, egui::RichText::new("● PEER : WAITING").monospace());
            }
            for peer in shown {
                ui.colored_label(palette.text, egui::RichText::new(format!("● PEER  {peer}")).monospace());
            }
            for (name, code) in &state.pending {
                ui.horizontal(|ui| {
                    ui.colored_label(palette.muted, egui::RichText::new(format!("● WAIT  {name}")).monospace());
                    if ui.button(talk_core::t("gui.button.verify", &[])).clicked() {
                        verify_open = Some((name.clone(), code.clone()));
                    }
                });
            }
        });
        if verify_open.is_some() {
            self.verify = verify_open;
        }
        // The bottom line as Python shows it: "[host] peers · mode · room ID · setup code", then only a relay failure (never a success text).
        let people = if supabase { state.room_names.len() } else { state.peers.len() }; // the room's people in Supabase mode
        let line = bottom_line(hosting, people, supabase, &state.room, &state.relay);
        egui::TopBottomPanel::bottom("terminal_status")
            .min_height(38.0)
            .frame(egui::Frame::new().fill(palette.background).stroke(egui::Stroke::new(1.0_f32, palette.border)).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
                let color = if status_kind == Kind::Problem { palette.danger } else { palette.accent };
                // The hint is placed first at the right edge; the status text then takes all the width that is left.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(palette.muted, egui::RichText::new(talk_core::t("chat.help_hint", &[])).monospace());
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(line).monospace().size(12.0).color(color)).wrap());
                    });
                });
            });
        egui::TopBottomPanel::bottom("input")
            .exact_height(52.0)
            .frame(egui::Frame::new().fill(palette.background).stroke(egui::Stroke::new(1.0_f32, palette.border)).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add_sized(
                    [ui.available_width() - 76.0, INPUT_HEIGHT],
                    egui::TextEdit::singleline(&mut self.input).hint_text("메시지를 입력하세요...").margin(egui::Margin::symmetric(6, 10)),
                );
                let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let clicked = ui.add_sized([68.0, INPUT_HEIGHT], egui::Button::new("SEND")).clicked();
                if (enter || clicked) && !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    match text.trim() {
                        // The window shows the phone address itself; the node only makes a new token.
                        "/phone" => self.phone_until = Some(Instant::now() + PHONE_SHOWN),
                        "/phone new" => {
                            let _ = self.outgoing.send(text);
                            self.phone_until = Some(Instant::now() + PHONE_SHOWN);
                        }
                        _ => {
                            let _ = self.outgoing.send(text);
                        }
                    }
                }
                if enter {
                    response.request_focus();
                }
            });
        });
        egui::CentralPanel::default().frame(inset(&palette, self.crt)).show(ctx, |ui| {
            ui.colored_label(palette.accent, egui::RichText::new(talk_core::t("gui.log.title", &[])).monospace().heading());
            ui.separator();
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for (text, mine) in &state.lines {
                    let color = if *mine { palette.mine } else { palette.text };
                    ui.colored_label(color, text);
                }
            });
        });
        if self.help_open {
            if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.help_open = false;
            }
            egui::Window::new(talk_core::t("help.keys", &[])).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                for key in HELP_KEYS {
                    if key.is_empty() {
                        ui.add_space(8.0);
                    } else {
                        ui.label(egui::RichText::new(talk_core::t(key, &[])).monospace());
                    }
                }
                ui.add_space(8.0);
                if ui.button(talk_core::t("gui.button.close", &[])).clicked() {
                    self.help_open = false;
                }
            });
        }
        // The verify window: show the peer's code to compare; 확인 trusts it (the same /join as typing), 닫기 leaves it.
        if let Some((name, code)) = self.verify.clone() {
            let (mut confirm, mut close) = (false, ctx.input(|input| input.key_pressed(egui::Key::Escape)));
            egui::Window::new(&name).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(talk_core::t("gui.verify.hint", &[]));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&code).monospace().size(28.0).color(palette.accent));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    confirm = ui.button(talk_core::t("gui.button.verify", &[])).clicked();
                    close |= ui.button(talk_core::t("gui.button.close", &[])).clicked();
                });
            });
            if confirm {
                let _ = self.outgoing.send(format!("/join {code}"));
            }
            if confirm || close {
                self.verify = None;
            }
        }
        if self.phone_until.is_some() {
            let mut rows = vec![egui::RichText::new(talk_core::t("phone.title", &[])).strong()];
            match talk_core::phone::read(&state_dir()) {
                Some(phone) => {
                    rows.extend(phone.urls.iter().map(|url| egui::RichText::new(url.as_str())));
                    rows.push(egui::RichText::new(talk_core::t("phone.token", &[("token", &phone.token)])).strong());
                    rows.push(egui::RichText::new(talk_core::t("phone.hint", &[])));
                }
                None => rows.push(egui::RichText::new(talk_core::t("phone.none", &[]))),
            }
            egui::Window::new("phone").title_bar(false).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -70.0]).show(ctx, |ui| {
                for row in rows {
                    ui.label(row);
                }
            });
        }
        if self.crt {
            draw_scanlines(ctx);
            draw_bezel(ctx);
        }
    }
}

/// Beige plastic bezel around the window, like an old CRT monitor's case: a dark outer edge and a light inner edge.
pub(crate) fn draw_bezel(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("crt_bezel")));
    let screen = ctx.screen_rect();
    let outer = egui::Color32::from_rgb(0x8a, 0x80, 0x6e);
    let light = egui::Color32::from_rgb(0xd8, 0xcf, 0xb8);
    let shadow = egui::Color32::from_rgb(0x4e, 0x48, 0x3c);
    painter.rect_stroke(screen.shrink(1.0), 0.0, egui::Stroke::new(2.0_f32, shadow), egui::StrokeKind::Inside);
    painter.rect_stroke(screen.shrink(3.0), 0.0, egui::Stroke::new(4.0_f32, outer), egui::StrokeKind::Inside);
    painter.rect_stroke(screen.shrink(6.0), 0.0, egui::Stroke::new(1.0_f32, light), egui::StrokeKind::Inside);
}

/// Dark line every 3 px over the whole window. Painted on a top layer; it does not take clicks or keys.
pub(crate) fn draw_scanlines(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("crt_scanlines")));
    let screen = ctx.screen_rect();
    let mut y = screen.top();
    while y < screen.bottom() {
        let line = egui::Rect::from_min_size(egui::pos2(screen.left(), y), egui::vec2(screen.width(), 1.0));
        painter.rect_filled(line, 0.0, egui::Color32::from_black_alpha(90));
        y += 3.0;
    }
}

