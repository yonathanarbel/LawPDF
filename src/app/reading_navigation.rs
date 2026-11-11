//! Keyboard scrolling shared by the original, Review, and split readers.

use eframe::egui::{self, Context, Key, Modifiers, Rect, Ui, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    LineUp,
    LineDown,
    LineLeft,
    LineRight,
    PageUp,
    PageDown,
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScrollCommand {
    direction: Direction,
    repeated: bool,
}

impl ScrollCommand {
    /// The Review/source split shows one complete original page at a time.
    pub(super) fn page_target(self, current: usize, count: usize) -> usize {
        let last = count.saturating_sub(1);
        match self.direction {
            Direction::Start => 0,
            Direction::End => last,
            Direction::LineUp | Direction::LineLeft | Direction::PageUp => {
                current.saturating_sub(1)
            }
            Direction::LineDown | Direction::LineRight | Direction::PageDown => {
                current.saturating_add(1).min(last)
            }
        }
    }
}

fn command_for_key(key: Key, modifiers: Modifiers, repeated: bool) -> Option<ScrollCommand> {
    if modifiers.alt || modifiers.mac_cmd && !modifiers.command {
        return None;
    }
    let command = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
    let direction = if command {
        if modifiers.shift {
            return None;
        }
        match key {
            Key::Home => Direction::Start,
            Key::End => Direction::End,
            _ => return None,
        }
    } else {
        match (key, modifiers.shift) {
            (Key::Space, true) => Direction::PageUp,
            (Key::Space | Key::PageDown, false) => Direction::PageDown,
            (Key::PageUp, false) => Direction::PageUp,
            (Key::ArrowDown | Key::J, false) => Direction::LineDown,
            (Key::ArrowUp | Key::K, false) => Direction::LineUp,
            (Key::ArrowLeft, false) => Direction::LineLeft,
            (Key::ArrowRight, false) => Direction::LineRight,
            (Key::Home, false) => Direction::Start,
            (Key::End, false) => Direction::End,
            _ => return None,
        }
    };
    Some(ScrollCommand {
        direction,
        repeated,
    })
}

/// Leave all input untouched when a text editor owns keyboard focus.
pub(super) fn consume(ctx: &Context, enabled: bool) -> Option<ScrollCommand> {
    if !enabled {
        return None;
    }
    ctx.input_mut(|input| {
        let mut command = None;
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                modifiers,
                repeat,
                pressed: true,
                ..
            } = event
            else {
                return true;
            };
            if let Some(next) = command_for_key(*key, *modifiers, *repeat) {
                command = Some(next);
                false
            } else {
                true
            }
        });
        command
    })
}

/// Called at the end of the target scroll area's contents, after its extent is known.
pub(super) fn apply(ui: &Ui, viewport: Rect, command: ScrollCommand, reduce_motion: bool) {
    let line = ui.text_style_height(&egui::TextStyle::Body).max(12.0);
    let page = (viewport.height() - 2.0 * line).max(line);
    let remaining = (ui.min_rect().height() - viewport.bottom()).max(0.0);
    // Egui's delta moves the content, so a negative y moves down the document.
    let delta = match command.direction {
        Direction::LineUp => Vec2::new(0.0, 3.0 * line),
        Direction::LineDown => Vec2::new(0.0, -3.0 * line),
        Direction::LineLeft => Vec2::new(3.0 * line, 0.0),
        Direction::LineRight => Vec2::new(-3.0 * line, 0.0),
        Direction::PageUp => Vec2::new(0.0, page),
        Direction::PageDown => Vec2::new(0.0, -page),
        Direction::Start => Vec2::new(0.0, viewport.top()),
        Direction::End => Vec2::new(0.0, -remaining),
    };
    let animation = if reduce_motion || command.repeated {
        egui::style::ScrollAnimation::none()
    } else {
        egui::style::ScrollAnimation::duration(super::motion::PAGING_SECONDS)
    };
    ui.scroll_with_delta_animation(delta, animation);
}

pub(super) fn draw_shortcuts(ctx: &Context, open: &mut bool) {
    if !*open {
        return;
    }
    let command = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    };
    egui::Window::new("Keyboard shortcuts")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("reading-shortcuts")
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    for (action, keys) in [
                        ("Scroll down / up", "↓ / ↑ or J / K".to_owned()),
                        ("Page down / up", "Page Down / Page Up".to_owned()),
                        ("Page down / up", "Space / Shift+Space".to_owned()),
                        ("Start / end of document", "Home / End".to_owned()),
                        ("Go to page number", format!("{command}G")),
                        ("Back / forward", "Alt+Left / Alt+Right".to_owned()),
                        (
                            "Fit page / actual size / fit width",
                            format!("{command}0 / {command}1 / {command}2"),
                        ),
                        ("Open document", format!("{command}O")),
                        ("Find", format!("{command}F")),
                        ("Copy selection", format!("{command}C")),
                        ("Copy selection with citation", format!("{command}Alt+C")),
                        ("Copy article as Markdown", format!("{command}Shift+C")),
                        ("Show this sheet", "?".to_owned()),
                    ] {
                        ui.label(action);
                        ui.label(keys);
                        ui.end_row();
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_keys_map_without_stealing_other_shortcuts() {
        let plain = Modifiers::NONE;
        let command = |key, mods| command_for_key(key, mods, false).map(|c| c.direction);
        assert_eq!(command(Key::PageDown, plain), Some(Direction::PageDown));
        assert_eq!(
            command(Key::Space, Modifiers::SHIFT),
            Some(Direction::PageUp)
        );
        assert_eq!(command(Key::J, plain), Some(Direction::LineDown));
        assert_eq!(command(Key::K, plain), Some(Direction::LineUp));
        assert_eq!(command(Key::Home, Modifiers::CTRL), Some(Direction::Start));
        assert_eq!(command(Key::End, Modifiers::CTRL), Some(Direction::End));
        assert_eq!(command(Key::ArrowLeft, Modifiers::ALT), None);
        assert_eq!(command(Key::J, Modifiers::CTRL), None);
    }

    #[test]
    fn source_pane_page_navigation_stays_in_bounds() {
        let start = command_for_key(Key::Home, Modifiers::NONE, false).unwrap();
        let end = command_for_key(Key::End, Modifiers::CTRL, false).unwrap();
        let down = command_for_key(Key::PageDown, Modifiers::NONE, false).unwrap();
        assert_eq!(start.page_target(3, 8), 0);
        assert_eq!(end.page_target(3, 8), 7);
        assert_eq!(down.page_target(7, 8), 7);
    }

    #[test]
    fn text_focus_leaves_reading_keys_for_the_editor() {
        let ctx = Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            assert!(consume(ctx, false).is_none());
            assert!(ctx.input(|input| !input.events.is_empty()));
            assert_eq!(consume(ctx, true).unwrap().direction, Direction::PageDown);
            assert!(ctx.input(|input| input.events.is_empty()));
        });
    }

    #[test]
    fn paging_changes_a_real_scroll_area_and_home_returns_to_start() {
        let ctx = Context::default();
        let mut offset = 0.0;
        for frame in 0..6 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(600.0, 400.0),
                )),
                time: Some(frame as f64 * 0.2),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let output = egui::ScrollArea::vertical()
                        .id_salt("test-reader")
                        .auto_shrink([false, false])
                        .show_viewport(ui, |ui, viewport| {
                            ui.add_space(2000.0);
                            if frame == 1 {
                                apply(
                                    ui,
                                    viewport,
                                    command_for_key(Key::PageDown, Modifiers::NONE, false).unwrap(),
                                    true,
                                );
                            } else if frame == 4 {
                                apply(
                                    ui,
                                    viewport,
                                    command_for_key(Key::Home, Modifiers::CTRL, false).unwrap(),
                                    true,
                                );
                            }
                        });
                    offset = output.state.offset.y;
                });
            });
            if frame == 3 {
                assert!(
                    offset > 100.0,
                    "Page Down did not move the reader: {offset}"
                );
            }
        }
        assert!(
            offset < 1.0,
            "Ctrl+Home did not return to the start: {offset}"
        );
    }
}
