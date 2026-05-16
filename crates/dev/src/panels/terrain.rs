use bevy::prelude::*;
use bevy_egui::egui;
use voxel_map_engine::prelude::{TerrainBrushMode, TerrainBrushShape};

/// User-selected terrain brush settings shared by dev UI and client terrain input.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct TerrainBrushSettings {
    pub active: bool,
    pub shape: TerrainBrushShape,
    pub width: u32,
    pub height: u32,
    pub material: u8,
    pub mode: TerrainBrushMode,
}

impl Default for TerrainBrushSettings {
    fn default() -> Self {
        Self {
            active: false,
            shape: TerrainBrushShape::Rect,
            width: 1,
            height: 1,
            material: 0,
            mode: TerrainBrushMode::FillAir,
        }
    }
}

/// Initializes terrain sculpting UI resources.
pub struct TerrainPanelPlugin;

impl Plugin for TerrainPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerrainBrushSettings>();
    }
}

/// Draws terrain brush controls inside the world-object panel's terrain tab.
pub fn draw_terrain_controls(ui: &mut egui::Ui, settings: &mut TerrainBrushSettings) {
    ui.checkbox(&mut settings.active, "Brush active");
    ui.add_enabled_ui(settings.active, |ui| {
        egui::Grid::new("terrain_brush_grid")
            .num_columns(2)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                ui.label("Shape");
                egui::ComboBox::from_id_salt("terrain_brush_shape")
                    .selected_text(format!("{:?}", settings.shape))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.shape, TerrainBrushShape::Rect, "Rect");
                        ui.selectable_value(
                            &mut settings.shape,
                            TerrainBrushShape::Sphere,
                            "Sphere",
                        );
                    });
                ui.end_row();

                ui.label("Mode");
                egui::ComboBox::from_id_salt("terrain_brush_mode")
                    .selected_text(format!("{:?}", settings.mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut settings.mode,
                            TerrainBrushMode::FillAir,
                            "Fill Air",
                        );
                        ui.selectable_value(&mut settings.mode, TerrainBrushMode::Remove, "Remove");
                        ui.selectable_value(
                            &mut settings.mode,
                            TerrainBrushMode::PaintExisting,
                            "Paint Existing",
                        );
                        ui.selectable_value(
                            &mut settings.mode,
                            TerrainBrushMode::ReplaceAll,
                            "Replace All",
                        );
                    });
                ui.end_row();

                ui.label("Width");
                draw_u32_stepper(ui, "terrain_brush_width", &mut settings.width, 1, 16);
                ui.end_row();

                ui.label("Height");
                draw_u32_stepper(ui, "terrain_brush_height", &mut settings.height, 1, 16);
                ui.end_row();

                ui.label("Material");
                draw_u8_stepper(
                    ui,
                    "terrain_brush_material",
                    &mut settings.material,
                    0,
                    u8::MAX,
                );
                ui.end_row();
            });
    });
}

fn draw_u32_stepper(ui: &mut egui::Ui, id: &'static str, value: &mut u32, min: u32, max: u32) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("-").clicked() {
                *value = value.saturating_sub(1).max(min);
            }
            ui.add(egui::DragValue::new(value).range(min..=max));
            if ui.small_button("+").clicked() {
                *value = value.saturating_add(1).min(max);
            }
        });
    });
}

fn draw_u8_stepper(ui: &mut egui::Ui, id: &'static str, value: &mut u8, min: u8, max: u8) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("-").clicked() {
                *value = value.saturating_sub(1).max(min);
            }
            ui.add(egui::DragValue::new(value).range(min..=max));
            if ui.small_button("+").clicked() {
                *value = value.saturating_add(1).min(max);
            }
        });
    });
}
