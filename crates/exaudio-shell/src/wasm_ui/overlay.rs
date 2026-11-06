use super::*;
fn color(value: &w::Color) -> Hsla {
    rgba(
        ((value.red as u32) << 24)
            | ((value.green as u32) << 16)
            | ((value.blue as u32) << 8)
            | value.alpha as u32,
    )
    .into()
}
fn location(
    point: &w::Point,
    bounds: Bounds<Pixels>,
    lane: usize,
    start: f64,
    end: f64,
) -> Point<Pixels> {
    point_fn(
        bounds.origin.x
            + px(
                ((point.time_ms - start) / (end - start).max(1.) * f64::from(bounds.size.width))
                    as f32,
            ),
        bounds.origin.y + px((point.lane - lane as f32) * f32::from(bounds.size.height)),
    )
}
fn point_fn(x: Pixels, y: Pixels) -> Point<Pixels> {
    point(x, y)
}
fn stroke(
    points: impl Iterator<Item = Point<Pixels>>,
    width: f32,
    color: Hsla,
    window: &mut Window,
) {
    let mut points = points;
    let Some(start) = points.next() else {
        return;
    };
    let mut builder = PathBuilder::stroke(px(width.clamp(0.5, 64.)));
    builder.move_to(start);
    for point in points {
        builder.line_to(point);
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}
/// Paints in the actual lane surface. `lane_index` is the stable project lane index,
/// not the filtered/visible row index; fractional WIT lane coordinates span its height.
pub fn timeline_overlay(
    document_id: &str,
    revision: u64,
    lane_index: usize,
    start_ms: f64,
    end_ms: f64,
    cx: &mut App,
) -> AnyElement {
    if !cx.has_global::<UiStore>() {
        return div().into_any_element();
    }
    let overlays = cx
        .global::<UiStore>()
        .owners
        .iter()
        .flat_map(|(owner, state)| {
            state
                .overlays
                .values()
                .filter(move |overlay| {
                    overlay.document_id == document_id && overlay.revision == revision
                })
                .map(move |overlay| (owner.clone(), overlay.clone()))
        })
        .collect::<Vec<_>>();
    let mut surface = div().absolute().inset_0().size_full();
    for (owner, overlay) in overlays {
        let draws = overlay.draws.clone();
        surface = surface.child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for draw in &draws {
                            match draw {
                                w::Draw::Line(line) => stroke(
                                    [
                                        location(&line.start, bounds, lane_index, start_ms, end_ms),
                                        location(&line.end, bounds, lane_index, start_ms, end_ms),
                                    ]
                                    .into_iter(),
                                    line.width,
                                    color(&line.color),
                                    window,
                                ),
                                w::Draw::Rect(rect) => {
                                    let start =
                                        location(&rect.start, bounds, lane_index, start_ms, end_ms);
                                    let end =
                                        location(&rect.end, bounds, lane_index, start_ms, end_ms);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            start,
                                            size(
                                                (end.x - start.x).max(px(0.)),
                                                (end.y - start.y).max(px(0.)),
                                            ),
                                        ),
                                        color(&rect.color),
                                    ));
                                }
                                w::Draw::Curve(curve) => stroke(
                                    curve
                                        .points
                                        .iter()
                                        .map(|p| location(p, bounds, lane_index, start_ms, end_ms)),
                                    curve.width,
                                    color(&curve.color),
                                    window,
                                ),
                                w::Draw::Label(label) => {
                                    let text: SharedString = label.text.clone().into();
                                    let run = TextRun {
                                        len: text.len(),
                                        font: window.text_style().font(),
                                        color: color(&label.color),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };
                                    let line = window.text_system().shape_line(
                                        text,
                                        px(12.),
                                        &[run],
                                        None,
                                    );
                                    let _ = line.paint(
                                        location(&label.at, bounds, lane_index, start_ms, end_ms),
                                        px(16.),
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                                w::Draw::HitRegion(_) => {}
                            }
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        );
        for draw in overlay.draws {
            if let w::Draw::HitRegion(region) = draw {
                let top = (region.start.lane - lane_index as f32).clamp(0., 1.);
                let bottom = (region.end.lane - lane_index as f32).clamp(0., 1.);
                let left = ((region.start.time_ms - start_ms) / (end_ms - start_ms).max(1.))
                    .clamp(0., 1.) as f32;
                let right = ((region.end.time_ms - start_ms) / (end_ms - start_ms).max(1.))
                    .clamp(0., 1.) as f32;
                if right <= left || bottom <= top {
                    continue;
                }
                let event = w::UiEvent {
                    contribution: overlay.provider.clone(),
                    node: region.id.clone(),
                    kind: w::UiEventKind::Click,
                    value: String::new(),
                    document_id: Some(overlay.document_id.clone()),
                };
                let click_owner = owner.clone();
                let hover_owner = owner.clone();
                let hover_event = event.clone();
                surface = surface.child(
                    div()
                        .id(SharedString::from(format!(
                            "wasm-hit:{owner}:{}:{}",
                            overlay.provider, region.id
                        )))
                        .absolute()
                        .left(relative(left))
                        .top(relative(top))
                        .w(relative(right - left))
                        .h(relative(bottom - top))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            crate::wasm_host::invoke(
                                &click_owner,
                                Invocation::Ui(event.clone()),
                                cx,
                            );
                            cx.stop_propagation();
                        })
                        .on_hover(move |hovered, _, cx| {
                            let mut event = hover_event.clone();
                            event.kind = w::UiEventKind::Hover;
                            event.value = hovered.to_string();
                            crate::wasm_host::invoke(&hover_owner, Invocation::Ui(event), cx);
                        }),
                );
            }
        }
    }
    surface.into_any_element()
}
