//! In-place block edit session: nested stack, save/discard, transforms.

use cad_core::{
    count_block_references, insert_transform, is_user_editable_block_name, BlockDefinition,
    Document, Entity, EntityId, EntitySpace, Geometry, Transform2,
};
use cad_render::{BlockEditView, BlockEditViewFrame};
use eframe::egui::{self, Align2, Color32, RichText, Ui};

use crate::history::History;

// ------------------------------------------------------------
// Type: BlockEditFrame
// Purpose: One nested level of in-place block editing.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct BlockEditFrame {
    pub block_name: String,
    pub instance_id: EntityId,
    pub parent_space: EntitySpace,
    pub world_from_local: Transform2,
    pub baseline: BlockDefinition,
    pub dirty: bool,
    pub undo_mark: usize,
}

// ------------------------------------------------------------
// Type: BlockEditSession
// Purpose: Nested edit stack and pending UI for create/save/leave.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct BlockEditSession {
    pub stack: Vec<BlockEditFrame>,
    pub ui: BlockUi,
}

#[derive(Debug, Clone, Default)]
pub enum BlockUi {
    #[default]
    None,
    Create(CreateBlockDialog),
    PickBase(CreateBlockDialog),
    LeaveDirty {
        name: String,
        intent: LeaveIntent,
    },
    AddConfirm {
        name: String,
        references: usize,
        ids: Vec<EntityId>,
    },
    SaveDrawing,
}

#[derive(Debug, Clone)]
pub struct CreateBlockDialog {
    pub name: String,
    pub base_x: String,
    pub base_y: String,
    pub replace: bool,
    pub ids: Vec<EntityId>,
    pub space: EntitySpace,
    pub error: Option<String>,
    pub just_opened: bool,
}

impl CreateBlockDialog {
    pub fn skip_open_frame(&mut self) -> bool {
        let skip = self.just_opened;
        self.just_opened = false;
        skip
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaveIntent {
    CloseOne,
    CloseTo(usize),
    OpenDrawing,
    Quit,
    EnterNested { instance_id: EntityId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    None,
    AddToBlock,
    RemoveFromBlock,
    Save,
    SaveAndClose,
    DiscardAndClose,
    Breadcrumb(usize),
}

impl BlockEditSession {
    pub fn is_active(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn current(&self) -> Option<&BlockEditFrame> {
        self.stack.last()
    }

    pub fn current_mut(&mut self) -> Option<&mut BlockEditFrame> {
        self.stack.last_mut()
    }

    pub fn active_space(&self) -> EntitySpace {
        self.current()
            .map(|frame| EntitySpace::Block(frame.block_name.clone()))
            .unwrap_or(EntitySpace::ModelSpace)
    }

    pub fn world_from_local(&self) -> Transform2 {
        self.current()
            .map(|frame| frame.world_from_local)
            .unwrap_or_else(Transform2::identity)
    }

    pub fn local_from_world(&self) -> Transform2 {
        self.world_from_local()
            .inverse()
            .unwrap_or_else(Transform2::identity)
    }

    pub fn undo_mark(&self) -> usize {
        self.current().map(|frame| frame.undo_mark).unwrap_or(0)
    }

    pub fn current_is_dirty(&self) -> bool {
        self.current().is_some_and(|frame| frame.dirty)
    }

    pub fn tess_view(&self) -> Option<BlockEditView> {
        if self.stack.is_empty() {
            return None;
        }
        Some(BlockEditView {
            frames: self
                .stack
                .iter()
                .map(|frame| BlockEditViewFrame {
                    instance_id: frame.instance_id,
                    block_name: frame.block_name.clone(),
                })
                .collect(),
        })
    }

    pub fn breadcrumb(&self) -> Vec<String> {
        let mut names = vec!["Model".to_string()];
        names.extend(self.stack.iter().map(|frame| frame.block_name.clone()));
        names
    }

    pub fn refresh_dirty(&mut self, document: &Document) {
        for frame in &mut self.stack {
            frame.dirty = document.block_by_name(&frame.block_name) != Some(&frame.baseline);
        }
    }

    pub fn space_world_from_local(&self, document: &Document, space: &EntitySpace) -> Transform2 {
        match space {
            EntitySpace::ModelSpace => Transform2::identity(),
            EntitySpace::Block(name) => self
                .stack
                .iter()
                .rev()
                .find(|frame| frame.block_name.eq_ignore_ascii_case(name))
                .map(|frame| frame.world_from_local)
                .or_else(|| document.block_by_name(name).map(|_| Transform2::identity()))
                .unwrap_or_else(Transform2::identity),
        }
    }

    pub fn enter(
        &mut self,
        document: &Document,
        history: &History,
        instance_id: EntityId,
    ) -> Result<(), String> {
        let Some(entity) = document.entity_by_id(instance_id) else {
            return Err("Select a block reference".into());
        };
        let Geometry::Insert { block_name, .. } = &entity.geometry else {
            return Err("Select a block reference".into());
        };
        if !is_user_editable_block_name(block_name) {
            return Err("Cannot edit a system block".into());
        }
        if !self.stack.is_empty() {
            if document
                .block_by_name(block_name)
                .is_some_and(|definition| definition.is_dynamic())
            {
                return Err(
                    "Editing a nested dynamic reference independently is not supported yet".into(),
                );
            }
        }
        if self
            .stack
            .iter()
            .any(|frame| frame.block_name.eq_ignore_ascii_case(block_name))
        {
            return Err("Cannot enter a circular block reference".into());
        }
        let definition = document
            .block_by_name(block_name)
            .cloned()
            .ok_or_else(|| format!("Block '{block_name}' was not found"))?;
        let parent_space = document
            .find_entity_location(instance_id)
            .map(|(space, _)| space)
            .unwrap_or(EntitySpace::ModelSpace);
        let parent_world = self.space_world_from_local(document, &parent_space);
        let local = insert_transform(document, entity)
            .ok_or_else(|| "Block transform is not available".to_string())?;
        let world_from_local = parent_world.then(local);
        self.stack.push(BlockEditFrame {
            block_name: definition.name.clone(),
            instance_id,
            parent_space,
            world_from_local,
            baseline: definition,
            dirty: false,
            undo_mark: history.undo_len(),
        });
        self.ui = BlockUi::None;
        Ok(())
    }

    pub fn pop(&mut self) {
        self.stack.pop();
        self.ui = BlockUi::None;
    }

    pub fn clear(&mut self) {
        self.stack.clear();
        self.ui = BlockUi::None;
    }

    pub fn on_block_renamed(&mut self, from: &str, to: &str) {
        for frame in &mut self.stack {
            if frame.block_name.eq_ignore_ascii_case(from) {
                frame.block_name = to.to_string();
                frame.baseline.name = to.to_string();
            }
            if let EntitySpace::Block(name) = &mut frame.parent_space {
                if name.eq_ignore_ascii_case(from) {
                    *name = to.to_string();
                }
            }
            rewrite_baseline_block_names(&mut frame.baseline.entities, from, to);
        }
        match &mut self.ui {
            BlockUi::LeaveDirty { name, .. } | BlockUi::AddConfirm { name, .. }
                if name.eq_ignore_ascii_case(from) =>
            {
                *name = to.to_string();
            }
            _ => {}
        }
    }

    pub fn save_current(
        &mut self,
        document: &Document,
        history: &mut History,
    ) -> Result<(), String> {
        let frame = self
            .current()
            .ok_or_else(|| "Not editing a block".to_string())?;
        let name = frame.block_name.clone();
        let mark = frame.undo_mark;
        let after = document
            .block_by_name(&name)
            .cloned()
            .ok_or_else(|| format!("Block '{name}' was not found"))?;
        if let Some(dynamic) = after.dynamic.as_ref() {
            cad_core::validate_definition(dynamic, &after.entities)
                .map_err(|err| err.to_string())?;
        }
        history.collapse_since(mark);
        if let Some(frame) = self.current_mut() {
            frame.baseline = after;
            frame.dirty = false;
            frame.undo_mark = history.undo_len();
        }
        Ok(())
    }

    pub fn discard_current(&mut self, document: &mut Document, history: &mut History) {
        let Some(frame) = self.current() else {
            return;
        };
        let mark = frame.undo_mark;
        history.discard_since(document, mark);
        if let Some(frame) = self.current_mut() {
            frame.dirty = false;
        }
    }

    pub fn request_leave(&mut self, intent: LeaveIntent) -> bool {
        if self.current_is_dirty() {
            let name = self
                .current()
                .map(|frame| frame.block_name.clone())
                .unwrap_or_default();
            self.ui = BlockUi::LeaveDirty { name, intent };
            false
        } else {
            true
        }
    }
}

pub fn insert_is_editable(entity: &Entity) -> bool {
    match &entity.geometry {
        Geometry::Insert { block_name, .. } => is_user_editable_block_name(block_name),
        _ => false,
    }
}

fn rewrite_baseline_block_names(entities: &mut [Entity], from: &str, to: &str) {
    for entity in entities {
        match &mut entity.geometry {
            Geometry::Insert { block_name, .. } | Geometry::Dimension { block_name } => {
                if block_name.eq_ignore_ascii_case(from) {
                    *block_name = to.to_string();
                }
            }
            _ => {}
        }
    }
}

pub fn show_toolbar(
    ui: &mut Ui,
    session: &BlockEditSession,
    document: &Document,
    can_add: bool,
    can_remove: bool,
) -> ToolbarAction {
    let Some(frame) = session.current() else {
        return ToolbarAction::None;
    };
    let mut action = ToolbarAction::None;
    let references = count_block_references(document, &frame.block_name);
    egui::Frame::new()
        .fill(Color32::from_rgb(28, 38, 36))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("BLOCK EDIT")
                        .strong()
                        .size(12.0)
                        .color(Color32::from_rgb(120, 210, 180)),
                );
                ui.separator();
                let crumbs = session.breadcrumb();
                for (index, name) in crumbs.iter().enumerate() {
                    if index > 0 {
                        ui.label(RichText::new(">").color(Color32::from_rgb(110, 128, 124)));
                    }
                    let current = index + 1 == crumbs.len();
                    let text = if current {
                        RichText::new(name)
                            .strong()
                            .color(Color32::from_rgb(230, 236, 232))
                    } else {
                        RichText::new(name).color(Color32::from_rgb(160, 180, 176))
                    };
                    if ui.add(egui::Button::new(text).frame(false)).clicked() && !current {
                        action = ToolbarAction::Breadcrumb(index);
                    }
                }
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{references} reference{}",
                        if references == 1 { "" } else { "s" }
                    ))
                    .color(Color32::from_rgb(150, 170, 166)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Discard & Close").clicked() {
                        action = ToolbarAction::DiscardAndClose;
                    }
                    if ui.button("Save & Close").clicked() {
                        action = ToolbarAction::SaveAndClose;
                    }
                    if ui
                        .add_enabled(frame.dirty, egui::Button::new("Save"))
                        .clicked()
                    {
                        action = ToolbarAction::Save;
                    }
                    ui.separator();
                    if ui
                        .add_enabled(can_remove, egui::Button::new("Remove from Block"))
                        .clicked()
                    {
                        action = ToolbarAction::RemoveFromBlock;
                    }
                    if ui
                        .add_enabled(can_add, egui::Button::new("Add to Block"))
                        .clicked()
                    {
                        action = ToolbarAction::AddToBlock;
                    }
                });
            });
        });
    action
}

pub fn show_create_dialog(
    ctx: &egui::Context,
    dialog: &mut CreateBlockDialog,
) -> CreateDialogResult {
    let mut result = CreateDialogResult::Open;
    egui::Window::new("Create Block")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Name");
            ui.add(
                egui::TextEdit::singleline(&mut dialog.name)
                    .desired_width(260.0)
                    .hint_text("Block_001"),
            );
            ui.add_space(8.0);
            ui.label("Base Point");
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(egui::TextEdit::singleline(&mut dialog.base_x).desired_width(88.0));
                ui.label("Y");
                ui.add(egui::TextEdit::singleline(&mut dialog.base_y).desired_width(88.0));
                if ui.button("Pick Point").clicked() {
                    result = CreateDialogResult::PickPoint;
                }
            });
            ui.add_space(8.0);
            ui.checkbox(
                &mut dialog.replace,
                "Replace selected objects with block instance",
            );
            if let Some(error) = &dialog.error {
                ui.add_space(6.0);
                ui.colored_label(Color32::from_rgb(220, 120, 90), error);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = CreateDialogResult::Cancel;
                }
                if ui.button("Create").clicked() {
                    result = CreateDialogResult::Create;
                }
            });
        });
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateDialogResult {
    Open,
    Create,
    Cancel,
    PickPoint,
}

pub fn show_leave_dialog(ctx: &egui::Context, name: &str) -> Option<LeaveChoice> {
    let mut choice = None;
    egui::Window::new("Unsaved block changes")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("Save changes to block \"{name}\"?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(LeaveChoice::Save);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(LeaveChoice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(LeaveChoice::Cancel);
                }
            });
        });
    choice
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveChoice {
    Save,
    Discard,
    Cancel,
}

pub fn show_add_confirm_dialog(ctx: &egui::Context, name: &str, references: usize) -> Option<bool> {
    let mut choice = None;
    egui::Window::new("Add to block")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "Adding this object to \"{name}\" will make it appear in all {references} references."
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
    choice
}

pub fn show_save_drawing_dialog(ctx: &egui::Context) -> Option<bool> {
    let mut choice = None;
    egui::Window::new("Unsaved block changes")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Save block changes and continue saving the drawing?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button("Save Block Changes & Continue Saving Drawing")
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
    choice
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        create_block_from_entities, default_extrusion, Document, Entity, EntitySpace, Point2,
        Point3,
    };

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    #[test]
    fn nested_stack_tracks_world_from_local() {
        let mut document = Document::default();
        let inner = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[inner.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let motor_insert = document.model_space[0].clone();
        let mut motor_at = motor_insert.clone();
        if let Geometry::Insert { insertion, .. } = &mut motor_at.geometry {
            *insertion = Point3::from_xy(10.0, 0.0);
        }
        motor_at.id = EntityId::UNASSIGNED;
        let nested_line = document.add_entity(line(5.0, 0.0, 6.0, 0.0));
        let motor_in_machine = document.add_entity(motor_at);
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[nested_line.id, motor_in_machine.id],
            "Machine",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let machine_id = document
            .model_space
            .iter()
            .find(|entity| {
                matches!(
                    &entity.geometry,
                    Geometry::Insert { block_name, .. } if block_name == "Machine"
                )
            })
            .expect("Machine INSERT")
            .id;
        let mut session = BlockEditSession::default();
        let history = History::default();
        session.enter(&document, &history, machine_id).unwrap();
        assert_eq!(session.breadcrumb(), ["Model", "Machine"]);
        let motor_id = document
            .block_by_name("Machine")
            .unwrap()
            .entities
            .iter()
            .find(|entity| matches!(entity.geometry, Geometry::Insert { .. }))
            .unwrap()
            .id;
        session.enter(&document, &history, motor_id).unwrap();
        assert_eq!(session.breadcrumb(), ["Model", "Machine", "Motor"]);
        let world = session.world_from_local().apply(Point2::new(1.0, 0.0));
        assert!((world.x - 11.0).abs() < 1e-9);
        assert!(world.y.abs() < 1e-9);
    }

    #[test]
    fn system_dimension_blocks_are_not_editable() {
        let entity = Entity::new(Geometry::Insert {
            block_name: "*D12".into(),
            insertion: Point3::from_xy(0.0, 0.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
        });
        assert!(!insert_is_editable(&entity));
    }

    #[test]
    fn save_packages_session_and_discard_restores_definition() {
        let mut document = Document::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let insert_id = document.model_space[0].id;
        let mut history = History::default();
        let mut session = BlockEditSession::default();
        session.enter(&document, &history, insert_id).unwrap();

        let space = EntitySpace::Block("Motor".into());
        history.begin();
        let extra = document
            .add_entity_to(&space, line(2.0, 0.0, 3.0, 0.0))
            .unwrap();
        history.record(crate::history::Edit::InsertEntity {
            space: space.clone(),
            index: 1,
            entity: extra.clone(),
        });
        history.commit_open();
        session.refresh_dirty(&document);
        assert!(session.current_is_dirty());

        session.discard_current(&mut document, &mut history);
        session.refresh_dirty(&document);
        assert!(!session.current_is_dirty());
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 1);

        history.begin();
        let extra = document
            .add_entity_to(&space, line(2.0, 0.0, 3.0, 0.0))
            .unwrap();
        history.record(crate::history::Edit::InsertEntity {
            space,
            index: 1,
            entity: extra,
        });
        history.commit_open();
        session.refresh_dirty(&document);
        session.save_current(&document, &mut history).unwrap();
        assert!(!session.current_is_dirty());
        assert!(history.is_dirty());
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 2);
        session.pop();
        assert!(history.undo(&mut document));
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 1);
    }

    #[test]
    fn rename_updates_session_path_and_baseline_inserts() {
        let mut document = Document::default();
        let inner = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[inner.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let motor_insert = document.model_space[0].clone();
        let nested_line = document.add_entity(line(5.0, 0.0, 6.0, 0.0));
        let mut nested_insert = motor_insert.clone();
        nested_insert.id = EntityId::UNASSIGNED;
        let motor_in_machine = document.add_entity(nested_insert);
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[nested_line.id, motor_in_machine.id],
            "Machine",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let machine_id = document
            .model_space
            .iter()
            .find(|entity| {
                matches!(
                    &entity.geometry,
                    Geometry::Insert { block_name, .. } if block_name == "Machine"
                )
            })
            .expect("Machine INSERT")
            .id;
        let mut session = BlockEditSession::default();
        let history = History::default();
        session.enter(&document, &history, machine_id).unwrap();
        let motor_id = document
            .block_by_name("Machine")
            .unwrap()
            .entities
            .iter()
            .find(|entity| matches!(entity.geometry, Geometry::Insert { .. }))
            .unwrap()
            .id;
        session.enter(&document, &history, motor_id).unwrap();
        session.on_block_renamed("Motor", "Motor Drive");
        assert_eq!(session.breadcrumb(), ["Model", "Machine", "Motor Drive"]);
        let parent = session.stack.first().unwrap();
        assert!(parent.baseline.entities.iter().any(|entity| matches!(
            &entity.geometry,
            Geometry::Insert { block_name, .. } if block_name == "Motor Drive"
        )));
    }

    #[test]
    fn create_dialog_skips_the_opening_frame() {
        let mut dialog = CreateBlockDialog {
            name: String::new(),
            base_x: "0".into(),
            base_y: "0".into(),
            replace: true,
            ids: Vec::new(),
            space: EntitySpace::ModelSpace,
            error: None,
            just_opened: true,
        };
        assert!(dialog.skip_open_frame());
        assert!(!dialog.just_opened);
        assert!(!dialog.skip_open_frame());
    }
}
