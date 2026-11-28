//! A fitted image opens centred. Manual zoom keeps the image point under
//! the pointer stationary wherever the edges of the image allow it.
use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{
    AnyElement, Context, Entity, IntoElement, MouseButton, ObjectFit, Pixels, Point, Resource,
    Role, ScrollEvent, Size, StyledImage as _, TestSupportExt as _, Window, container_query, div,
    img, point, prelude::*, px,
};

use super::{
    File, Stage,
    render::{path_key, unsupported},
};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Zoom {
    /// Physical image pixels per display pixel are equal at 1.0.
    scale: Option<f32>,
    offset: Point<Pixels>,
    drag: Option<Point<Pixels>>,
}

#[derive(Clone, Copy)]
struct Frame {
    viewport: Size<Pixels>,
    native: Size<Pixels>,
    fit: f32,
}

impl Frame {
    fn constrain(self, offset: Point<Pixels>, scale: f32) -> Point<Pixels> {
        let excess = self.native.map(|axis| axis * scale).center() - self.viewport.center();
        let bound = |delta: Pixels, limit: Pixels| {
            let limit = limit.max(px(0.));
            delta.clamp(-limit, limit)
        };
        point(bound(offset.x, excess.x), bound(offset.y, excess.y))
    }
}

impl Zoom {
    fn set_scale(&mut self, frame: Frame, scale: f32, anchor: Point<Pixels>) {
        let old = self.scale.unwrap_or(frame.fit);
        let new = scale.clamp(frame.fit.min(0.01), frame.fit.max(32.));
        let offset = frame.constrain(self.offset, old);
        self.offset = frame.constrain(anchor - (anchor - offset) * (new / old), new);
        self.scale = Some(new);
    }
}

impl Stage {
    pub(super) fn render_image(
        &self,
        file: &File,
        len: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resource = Resource::Path(Arc::clone(&file.path));
        let dimensions = self.images.read(cx).dimensions(&resource, window);
        let muted = cx.theme().muted_foreground;
        let name = file.name.clone();
        let picture = img(Arc::clone(&file.path))
            .image_cache(&self.images)
            .id(("image", path_key(&file.path)))
            .size_full()
            .object_fit(ObjectFit::Contain)
            .with_fallback(move || unsupported(name.clone(), Some(len), muted));
        let centre = Rc::new(Cell::new(Point::default()));
        let measured = Rc::clone(&centre);
        let viewport = div()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(child) = bounds.first() {
                    measured.set(child.center());
                }
            })
            .id("stage-image")
            .test_support()
            .role(Role::Group)
            .aria_label(file.name.clone())
            .absolute()
            .inset_0()
            .flex()
            .when(!self.asked.presenting, Styled::p_6);
        let Some(dimensions) = dimensions else {
            return viewport.child(picture).into_any_element();
        };
        let density = window.scale_factor();
        let native = dimensions.to_pixels(density);
        let entity = cx.entity();
        let zoom = self.zoom;
        let controls = !self.asked.presenting;
        let label = file.name.clone();
        viewport
            .child(container_query(move |size, window, cx| {
                let fit = (size.width / native.width)
                    .min(size.height / native.height)
                    .min(density);
                let frame = Frame {
                    viewport: size,
                    native,
                    fit,
                };
                let scale = zoom.scale.unwrap_or(fit);
                let drawn = native.map(|axis| axis * scale);
                let origin = size.center() - drawn.center() + frame.constrain(zoom.offset, scale);
                let surface = Self::image_surface(frame, drawn, centre, window, &entity);
                surface
                    .child(picture_at(origin, drawn, label, picture))
                    .when(controls && fit > 0., |el| {
                        el.child(toolbar(&entity, frame, scale, window, cx))
                    })
            }))
            .into_any_element()
    }
    fn image_surface(
        frame: Frame,
        drawn: Size<Pixels>,
        centre: Rc<Cell<Point<Pixels>>>,
        window: &Window,
        entity: &Entity<Self>,
    ) -> gpui_kit::Div {
        let fit = frame.fit;
        let can_pan = drawn.width > frame.viewport.width || drawn.height > frame.viewport.height;
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .when(can_pan, |el| el.cursor(gpui_kit::CursorStyle::OpenHand))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(
                    entity,
                    move |stage, event: &gpui_kit::MouseDownEvent, _, _| {
                        stage.zoom.drag = can_pan.then_some(event.position);
                    },
                ),
            )
            .on_scroll(
                window.listener_for(entity, move |stage, event: &ScrollEvent, _, cx| {
                    let delta = event.delta.pixel_delta(px(32.)).y;
                    if delta == px(0.) || fit <= 0. {
                        return;
                    }
                    let factor = 2_f32.powf(delta / px(128.));
                    stage.zoom.set_scale(
                        frame,
                        stage.zoom.scale.unwrap_or(fit) * factor,
                        event.position - centre.get(),
                    );
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(window.listener_for(
                entity,
                move |stage, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        stage.zoom.drag = None;
                        return;
                    }
                    if let Some(last) = stage.zoom.drag {
                        stage.zoom.offset = frame.constrain(
                            stage.zoom.offset + (event.position - last),
                            stage.zoom.scale.unwrap_or(fit),
                        );
                        stage.zoom.drag = Some(event.position);
                        cx.notify();
                    }
                },
            ))
    }
}

fn picture_at(
    origin: Point<Pixels>,
    drawn: Size<Pixels>,
    label: gpui_kit::SharedString,
    picture: impl IntoElement,
) -> impl IntoElement {
    div()
        .id("zoom-picture")
        .test_support()
        .role(Role::Image)
        .aria_label(label)
        .absolute()
        .flex()
        .left(origin.x)
        .top(origin.y)
        .w(drawn.width)
        .h(drawn.height)
        .child(picture)
}

fn toolbar(
    stage: &Entity<Stage>,
    frame: Frame,
    scale: f32,
    window: &Window,
    cx: &gpui_kit::App,
) -> impl IntoElement + use<> {
    let mut row = h_flex()
        .absolute()
        .bottom_1()
        .right_1()
        .rounded_md()
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border);
    for (id, label, tip, target) in [
        ("zoom-out", "−".to_owned(), "Zoom out", Some(scale / 1.25)),
        (
            "zoom-actual",
            format!("{:.0}%", scale * 100.),
            "Actual size (100%)",
            Some(1.),
        ),
        ("zoom-in", "+".to_owned(), "Zoom in", Some(scale * 1.25)),
        ("zoom-fit", "Fit".to_owned(), "Fit image to window", None),
    ] {
        row = row.child(
            Button::new(id)
                .label(label)
                .small()
                .compact()
                .ghost()
                .tooltip(tip)
                .accessibility_label(tip)
                .on_click(window.listener_for(stage, move |stage, _, _, cx| {
                    if let Some(scale) = target {
                        stage.zoom.set_scale(frame, scale, Point::default());
                    } else {
                        stage.zoom = Zoom::default();
                    }
                    cx.notify();
                })),
        );
    }
    row
}
