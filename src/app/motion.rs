//! Shared timing and easing for reader motion.
use eframe::egui::{self, Context};
use std::time::{Duration, Instant};

pub(super) const SURFACE_SECONDS: f32 = 0.18;
pub(super) const PAGING_SECONDS: f32 = 0.14;
pub(super) const REVEAL_SECONDS: f64 = 3.2;
pub(super) const PAGE_ARRIVAL: Duration = Duration::from_millis(120);
pub(super) const SAVE_CONFIRMATION: Duration = Duration::from_millis(300);
pub(super) const JUMP_SECONDS: f32 = 0.25;
const FRAME: Duration = Duration::from_millis(16);

pub(super) fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let remaining = 1.0 - t;
    1.0 - remaining * remaining * remaining
}

pub(super) fn ease_in_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

pub(super) fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub(super) fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Request frames only while a page is actually transitioning.
pub(super) fn page_opacity(ctx: &Context, started: Instant, reduce_motion: bool) -> f32 {
    if reduce_motion {
        return 1.0;
    }
    let elapsed = started.elapsed();
    if elapsed >= PAGE_ARRIVAL {
        return 1.0;
    }
    ctx.request_repaint_after(FRAME.min(PAGE_ARRIVAL - elapsed));
    ease_out_cubic(elapsed.as_secs_f32() / PAGE_ARRIVAL.as_secs_f32())
}

/// Animate nearby jumps for at most a quarter second. Distant jumps and all
/// jumps under Reduce motion land immediately rather than sweeping through the
/// intervening document. `distance` and `viewport_height` are in UI points.
pub(super) fn jump_animation(
    distance: f32,
    viewport_height: f32,
    reduce_motion: bool,
) -> egui::style::ScrollAnimation {
    if reduce_motion
        || !distance.is_finite()
        || !viewport_height.is_finite()
        || viewport_height <= 0.0
        || distance.abs() > 3.0 * viewport_height
    {
        egui::style::ScrollAnimation::none()
    } else {
        egui::style::ScrollAnimation::new(1200.0, egui::emath::Rangef::new(0.0, JUMP_SECONDS))
    }
}

/// Progress is tied to the actual successful save, never to a repaint or a
/// tab switch. A completed check mark does not keep the UI awake.
pub(super) fn save_progress(ctx: &Context, started: Instant, reduce_motion: bool) -> f32 {
    if reduce_motion {
        return 1.0;
    }
    let elapsed = started.elapsed();
    if elapsed >= SAVE_CONFIRMATION {
        return 1.0;
    }
    ctx.request_repaint_after(FRAME.min(SAVE_CONFIRMATION - elapsed));
    ease_out_cubic(elapsed.as_secs_f32() / SAVE_CONFIRMATION.as_secs_f32())
}

/// A static, accessible busy cue when animation is disabled.
pub(super) fn busy(ui: &mut egui::Ui, reduce_motion: bool) -> egui::Response {
    if reduce_motion {
        ui.label("Working…")
    } else {
        ui.spinner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_arrival_uses_the_thumbnail_only_during_motion_and_never_on_replacement() {
        use super::super::{PageTextureView, ThumbnailTexture, annotation_saving};
        use crate::render_worker::PreparedPage;
        use egui::{Color32, ColorImage, Pos2, Rect, TextureOptions, Vec2};
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        // The host's accessibility setting may default this to true. This
        // test exercises both choices explicitly and starts with motion on.
        app.settings.reduce_motion = false;
        app.settings.dim_original_pages = false;
        let image = || ColorImage::new([2, 2], vec![Color32::WHITE; 4]);
        let thumbnail = ctx.load_texture("motion-thumb", image(), TextureOptions::LINEAR);
        let page = ctx.load_texture("motion-page", image(), TextureOptions::LINEAR);
        app.thumbnail_textures.insert(
            0,
            ThumbnailTexture {
                texture: thumbnail.clone(),
                render_scale: 1.0,
            },
        );
        let view = PageTextureView {
            texture_id: page.id(),
            fade_started: Some(Instant::now() + Duration::from_secs(1)),
        };
        let draw = |app: &super::super::PdfEditorApp, ctx: &Context, use_thumbnail| {
            ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(100.0, 100.0));
                    app.paint_page_texture(ctx, ui.painter(), rect, 0, view, use_thumbnail);
                });
            })
        };
        let images = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh)
                        if mesh.texture_id == thumbnail.id() || mesh.texture_id == page.id() =>
                    {
                        Some((mesh.texture_id, mesh.vertices[0].color.a()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let animated = draw(&app, &ctx, true);
        assert_eq!(
            images(&animated),
            vec![(thumbnail.id(), 255), (page.id(), 0)]
        );
        app.settings.reduce_motion = true;
        assert_eq!(images(&draw(&app, &ctx, true)), vec![(page.id(), 255)]);
        app.settings.reduce_motion = false;
        assert_eq!(images(&draw(&app, &ctx, false)), vec![(page.id(), 255)]);
        let make_page = || PreparedPage {
            page_index: 0,
            width: 2,
            height: 2,
            image: image(),
        };
        let _ = ctx.run(Default::default(), |ctx| {
            app.install_page_texture(ctx, 7, make_page(), 1.0, 1.0)
        });
        assert!(app.page_textures[&0].fade_started.is_some());
        let _ = ctx.run(Default::default(), |ctx| {
            app.install_page_texture(ctx, 7, make_page(), 1.1, 1.1)
        });
        assert!(
            app.page_textures[&0].fade_started.is_none(),
            "a sharper replacement must not flash a thumbnail"
        );
        app.liquid_mode2_reveal_at = Some(10.0);
        app.settings.reduce_motion = true;
        assert_eq!(app.liquid_mode2_reveal_progress(10.0), 1.0);
    }

    #[test]
    fn original_page_dimming_only_changes_display_tint_for_page_and_placeholder() {
        use super::super::{PageTextureView, ThumbnailTexture, annotation_saving};
        use egui::{Color32, ColorImage, Pos2, Rect, TextureOptions, Vec2};
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.dim_original_pages = false;
        let image = || ColorImage::new([2, 2], vec![Color32::WHITE; 4]);
        let thumbnail = ctx.load_texture("dim-thumb", image(), TextureOptions::LINEAR);
        let page = ctx.load_texture("dim-page", image(), TextureOptions::LINEAR);
        app.thumbnail_textures.insert(
            0,
            ThumbnailTexture {
                texture: thumbnail.clone(),
                render_scale: 1.0,
            },
        );
        let draw = |app: &super::super::PdfEditorApp, placeholder| {
            ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(100.0, 100.0));
                    if placeholder {
                        app.draw_page_loading_placeholder(ctx, ui.painter(), rect, 0);
                    } else {
                        app.paint_page_texture(
                            ctx,
                            ui.painter(),
                            rect,
                            0,
                            PageTextureView {
                                texture_id: page.id(),
                                fade_started: None,
                            },
                            true,
                        );
                    }
                });
            })
        };
        let mesh_color = |output: &egui::FullOutput, texture| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                        Some(mesh.vertices[0].color)
                    }
                    _ => None,
                })
                .expect("original page image")
        };
        assert_eq!(mesh_color(&draw(&app, false), page.id()), Color32::WHITE);
        app.settings.dim_original_pages = true;
        let tint = Color32::from_rgb(190, 180, 162);
        assert_eq!(mesh_color(&draw(&app, false), page.id()), tint);
        assert_eq!(mesh_color(&draw(&app, true), thumbnail.id()), tint);
        app.settings.dim_original_pages = false;
        assert_eq!(mesh_color(&draw(&app, false), page.id()), Color32::WHITE);
    }

    #[test]
    fn eased_curves_stop_and_reduced_motion_never_schedules_animation() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_cubic(1.0), 1.0);
        assert_eq!(ease_in_out(0.5), 0.5);
        let ctx = Context::default();
        let past = Instant::now() - Duration::from_secs(1);
        for frame in 0..8 {
            let output = ctx.run(Default::default(), |ctx| {
                assert_eq!(page_opacity(ctx, past, false), 1.0);
                assert_eq!(page_opacity(ctx, Instant::now(), true), 1.0);
                egui::CentralPanel::default().show(ctx, |ui| {
                    busy(ui, true);
                });
            });
            if frame >= 4 {
                assert_eq!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                    Duration::MAX
                );
            }
        }
    }

    #[test]
    fn jump_duration_is_bounded_and_respects_reduce_motion() {
        let none = egui::style::ScrollAnimation::none();
        let nearby =
            egui::style::ScrollAnimation::new(1200.0, egui::emath::Rangef::new(0.0, JUMP_SECONDS));
        assert_eq!(jump_animation(140.0, 400.0, false), nearby);
        assert_eq!(jump_animation(-1200.0, 400.0, false), nearby);
        assert_eq!(jump_animation(1200.1, 400.0, false), none);
        assert_eq!(jump_animation(140.0, 400.0, true), none);
        assert_eq!(jump_animation(f32::NAN, 400.0, false), none);
        assert_eq!(jump_animation(1.0, 0.0, false), none);
    }

    #[test]
    fn egui_jump_scrolls_then_settles_and_reduce_motion_lands_immediately() {
        fn run(reduce: bool, target_y: f32) -> (f32, f32, Duration) {
            let ctx = Context::default();
            let frame = |time: f64, jump: bool| {
                let mut offset = 0.0;
                let output = ctx.run(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(500.0, 400.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let result = egui::ScrollArea::vertical()
                                .id_salt("motion-test")
                                .show_viewport(ui, |ui, _viewport| {
                                    ui.set_min_height(3000.0);
                                    if jump {
                                        let clip = ui.clip_rect();
                                        let target = egui::Rect::from_min_size(
                                            egui::pos2(
                                                ui.min_rect().left(),
                                                ui.min_rect().top() + target_y,
                                            ),
                                            egui::vec2(1.0, 1.0),
                                        );
                                        ui.scroll_to_rect_animation(
                                            target,
                                            Some(egui::Align::TOP),
                                            jump_animation(
                                                target.top() - clip.top(),
                                                clip.height(),
                                                reduce,
                                            ),
                                        );
                                    }
                                });
                            offset = result.state.offset.y;
                        });
                    },
                );
                (
                    offset,
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                )
            };
            for i in 0..5 {
                frame(i as f64 * 0.1, false);
            }
            frame(1.0, true);
            let early = frame(1.05, false).0;
            let end = frame(1.6, false).0;
            let mut delay = Duration::ZERO;
            for i in 0..8 {
                delay = frame(2.0 + i as f64 * 0.2, false).1;
            }
            (early, end, delay)
        }
        let (moving, end, delay) = run(false, 650.0);
        assert!(
            moving > 0.0 && moving < end,
            "nearby jump should animate: {moving} -> {end}"
        );
        assert!(end > 600.0);
        assert_eq!(delay, Duration::MAX);
        let (instant, reduced_end, reduced_delay) = run(true, 650.0);
        assert!((instant - reduced_end).abs() < 0.1);
        assert!(reduced_end > 600.0);
        assert_eq!(reduced_delay, Duration::MAX);
        let (far, far_end, _) = run(false, 2500.0);
        assert!(
            (far - far_end).abs() < 0.1,
            "distant jump should be immediate"
        );
    }

    #[test]
    fn save_confirmation_stops_repainting_and_reduce_motion_is_immediate() {
        let ctx = Context::default();
        let started = Instant::now();
        let _ = ctx.run(Default::default(), |ctx| {
            assert!(save_progress(ctx, started, false) < 1.0);
        });
        for frame in 0..8 {
            let output = ctx.run(Default::default(), |ctx| {
                assert_eq!(save_progress(ctx, started, true), 1.0);
                assert_eq!(
                    save_progress(ctx, Instant::now() - Duration::from_secs(1), false),
                    1.0
                );
            });
            if frame >= 4 {
                assert_eq!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                    Duration::MAX
                );
            }
        }
    }
}
