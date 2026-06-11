use bevy_egui::egui;

/// Equal left gutter width shared by ruler/dope sheet/curve/event lanes so the `t→x`
/// track rect aligns across all views.
pub const LANE_GUTTER: f32 = 96.0;

/// Computes the track `Rect` (gutter-excluded drawable area) from a lane's full `Rect`.
pub fn track_rect(lane: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(lane.left() + LANE_GUTTER, lane.top()), lane.max)
}

/// Draws a diamond (rotated square) centered at `pos` — the keyframe glyph used in every
/// lane. Selected diamonds get a contrasting outline.
pub fn draw_diamond(
    painter: &egui::Painter,
    pos: egui::Pos2,
    size: f32,
    color: egui::Color32,
    selected: bool,
) {
    let points = vec![
        egui::pos2(pos.x, pos.y - size),
        egui::pos2(pos.x + size, pos.y),
        egui::pos2(pos.x, pos.y + size),
        egui::pos2(pos.x - size, pos.y),
    ];
    let stroke = if selected {
        egui::Stroke::new(2.0, egui::Color32::WHITE)
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_gray(40))
    };
    painter.add(egui::Shape::convex_polygon(points, color, stroke));
}

/// Draws the shared playhead vertical line at `x` across a lane's rect.
pub fn draw_playhead(painter: &egui::Painter, x: f32, rect: egui::Rect) {
    painter.line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(1.5, egui::Color32::from_rgb(230, 80, 80)),
    );
}
