//! Annotation edits carry the waveform identity captured by the inspector.
use crate::{
    annotation::Annotatable,
    annotation_list::{AnnotationGroup, DEFAULT_GROUP_NAME},
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub enum AnnotationCommand {
    CreateGroup(String),
    DeleteGroup(String),
    DeleteGroupAnnotations(String),
    MoveToGroup {
        annotation: egui::Id,
        group: Option<String>,
    },
    Rename {
        annotation: egui::Id,
        name: String,
    },
    Remove(egui::Id),
    ToggleVisibility(egui::Id),
    GroupVisibility {
        group: String,
        visible: bool,
    },
    ToggleCommentBox(egui::Id),
    RemoveComment {
        annotation: egui::Id,
        comment: egui::Id,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AnnotationEditError {
    #[error("annotation no longer exists")]
    MissingAnnotation,
    #[error("annotation group no longer exists")]
    MissingGroup,
    #[error("annotation group name is empty or reserved")]
    InvalidGroup,
}

impl crate::item_list::ItemList {
    pub(crate) fn apply_annotation_command(
        &mut self,
        command: AnnotationCommand,
    ) -> Result<bool, AnnotationEditError> {
        use AnnotationCommand::*;
        match command {
            CreateGroup(name) => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(AnnotationEditError::InvalidGroup);
                }
                if self
                    .annotation_groups
                    .iter()
                    .any(|group| group.name == name)
                {
                    return Ok(false);
                }
                self.annotation_groups.push(AnnotationGroup {
                    name: name.into(),
                    annotations: Vec::new(),
                });
            }
            DeleteGroup(name) => {
                if name == DEFAULT_GROUP_NAME {
                    return Err(AnnotationEditError::InvalidGroup);
                }
                let index = self
                    .annotation_groups
                    .iter()
                    .position(|group| group.name == name)
                    .ok_or(AnnotationEditError::MissingGroup)?;
                let removed = self.annotation_groups.remove(index);
                if !self
                    .annotation_groups
                    .iter()
                    .any(|group| group.name == DEFAULT_GROUP_NAME)
                {
                    self.annotation_groups.push(AnnotationGroup {
                        name: DEFAULT_GROUP_NAME.into(),
                        annotations: Vec::new(),
                    });
                }
                let ungrouped = self
                    .annotation_groups
                    .iter_mut()
                    .find(|group| group.name == DEFAULT_GROUP_NAME)
                    .unwrap();
                for id in removed.annotations {
                    if !ungrouped.annotations.contains(&id) {
                        ungrouped.annotations.push(id);
                    }
                }
            }
            DeleteGroupAnnotations(name) => {
                let ids = self
                    .annotation_groups
                    .iter()
                    .find(|group| group.name == name)
                    .ok_or(AnnotationEditError::MissingGroup)?
                    .annotations
                    .clone();
                if ids.is_empty() {
                    return Ok(false);
                }
                self.annotations
                    .retain(|annotation| !ids.contains(&annotation.get_id()));
                for group in &mut self.annotation_groups {
                    group.annotations.retain(|id| !ids.contains(id));
                }
            }
            MoveToGroup { annotation, group } => {
                if self.get_annotation_by_id(&annotation).is_none() {
                    return Err(AnnotationEditError::MissingAnnotation);
                }
                let destination = group.as_deref().unwrap_or(DEFAULT_GROUP_NAME);
                let target = self
                    .annotation_groups
                    .iter()
                    .position(|group| group.name == destination)
                    .ok_or(AnnotationEditError::MissingGroup)?;
                if self.annotation_groups[target]
                    .annotations
                    .contains(&annotation)
                {
                    return Ok(false);
                }
                for group in &mut self.annotation_groups {
                    group.annotations.retain(|id| *id != annotation);
                }
                self.annotation_groups[target].annotations.push(annotation);
            }
            Remove(id) => {
                if self.get_annotation_by_id(&id).is_none() {
                    return Ok(false);
                }
                self.delete_annotation(id);
                for group in &mut self.annotation_groups {
                    group.annotations.retain(|other| *other != id);
                }
            }
            GroupVisibility { group, visible } => {
                let ids = &self
                    .annotation_groups
                    .iter()
                    .find(|entry| entry.name == group)
                    .ok_or(AnnotationEditError::MissingGroup)?
                    .annotations;
                let mut changed = false;
                for annotation in &mut self.annotations {
                    if ids.contains(&annotation.get_id()) && annotation.is_visible() != visible {
                        annotation.set_visibility(visible);
                        changed = true;
                    }
                }
                return Ok(changed);
            }
            Rename { annotation, name } => {
                let target = self
                    .annotations
                    .iter_mut()
                    .find(|entry| entry.get_id() == annotation)
                    .ok_or(AnnotationEditError::MissingAnnotation)?;
                if target.get_name() == name {
                    return Ok(false);
                }
                target.set_name(&name);
            }
            ToggleVisibility(id) => {
                let target = self
                    .annotations
                    .iter_mut()
                    .find(|entry| entry.get_id() == id)
                    .ok_or(AnnotationEditError::MissingAnnotation)?;
                target.set_visibility(!target.is_visible());
            }
            ToggleCommentBox(id) => {
                let target = self
                    .annotations
                    .iter_mut()
                    .find(|entry| entry.get_id() == id)
                    .ok_or(AnnotationEditError::MissingAnnotation)?;
                let comment = target.get_comment_box_mut();
                comment.visible = !comment.visible;
            }
            RemoveComment {
                annotation,
                comment,
            } => {
                let target = self
                    .annotations
                    .iter_mut()
                    .find(|entry| entry.get_id() == annotation)
                    .ok_or(AnnotationEditError::MissingAnnotation)?;
                let messages = &mut target.get_comment_box_mut().message_chain;
                let before = messages.len();
                messages.retain(|entry| entry.id != comment);
                return Ok(messages.len() != before);
            }
        }
        Ok(true)
    }
}

#[derive(Debug, Deserialize)]
pub enum AnnotationListMessage {
    ShowComments(bool),
}

#[derive(Clone, Default, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationListTile {
    pub show_comments: bool,
}

impl AnnotationListTile {
    pub(crate) fn update(&mut self, message: AnnotationListMessage) -> bool {
        match message {
            AnnotationListMessage::ShowComments(show) => {
                let changed = self.show_comments != show;
                self.show_comments = show;
                changed
            }
        }
    }
    pub(crate) fn ui(
        &self,
        ui: &mut egui::Ui,
        tile_id: crate::tiles::TileId,
        waves: Option<crate::wave_data::WaveformRead<'_>>,
        services: &super::waveform_services::WaveformReadServices<'_>,
        messages: &mut Vec<crate::Message>,
    ) {
        let mut show_comments = self.show_comments;
        if ui.checkbox(&mut show_comments, "Show comments").changed() {
            messages.push(crate::Message::ToTile(
                tile_id,
                crate::tiles::kind::TileMessage::AnnotationList(
                    AnnotationListMessage::ShowComments(show_comments),
                ),
            ));
        }
        let Some(waves) = waves else {
            ui.label("Open a waveform tile to inspect annotations.");
            return;
        };
        let time_formatter = crate::time::TimeFormatter::new(
            &waves.inner.metadata().timescale,
            &services.wanted_timeunit,
            &services.time_format,
        );
        ui.push_id(waves.tile_id, |ui| {
            waves.items.draw_annotation_list(
                ui,
                messages,
                &time_formatter,
                waves.tile_id,
                self.show_comments,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        annotation::Annotation,
        rectangle::RectAnnotation,
        tiles::{
            commands::WorkspaceCommand,
            kind::{TileKind, TileMessage},
            layout::{Direction, Placement},
            runtime::WorkspaceRuntime,
            workspace::Workspace,
        },
    };

    fn annotation() -> Annotation {
        Annotation::Rect(RectAnnotation::new(
            egui::Id::new("annotation"),
            10.into(),
            20.into(),
            None,
            None,
            egui::Rect::ZERO,
            1,
        ))
    }

    #[test]
    fn failed_group_move_preserves_membership_and_group_deletion_keeps_annotations() {
        let mut items = crate::item_list::ItemList::default();
        let annotation = annotation();
        let id = annotation.get_id();
        items.annotations.push(annotation);
        items.annotation_groups = vec![AnnotationGroup {
            name: "Source".into(),
            annotations: vec![id],
        }];
        assert!(matches!(
            items.apply_annotation_command(AnnotationCommand::MoveToGroup {
                annotation: id,
                group: Some("Missing".into()),
            }),
            Err(AnnotationEditError::MissingGroup)
        ));
        assert_eq!(items.annotation_groups[0].annotations, vec![id]);
        assert!(
            items
                .apply_annotation_command(AnnotationCommand::DeleteGroup("Source".into()))
                .unwrap()
        );
        assert_eq!(items.annotations.len(), 1);
        assert_eq!(items.annotation_groups[0].name, DEFAULT_GROUP_NAME);
        assert_eq!(items.annotation_groups[0].annotations, vec![id]);
        assert!(
            items
                .apply_annotation_command(AnnotationCommand::DeleteGroupAnnotations(
                    DEFAULT_GROUP_NAME.into()
                ))
                .unwrap()
        );
        assert!(items.annotations.is_empty());
        assert!(items.annotation_groups[0].annotations.is_empty());
        assert!(
            !items
                .apply_annotation_command(AnnotationCommand::DeleteGroupAnnotations(
                    DEFAULT_GROUP_NAME.into()
                ))
                .unwrap()
        );
    }

    #[test]
    fn captured_annotation_edit_does_not_follow_focus_or_retarget_after_close() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let mut ids = Vec::new();
        for _ in 0..2 {
            workspace
                .apply_command(
                    &mut runtime,
                    WorkspaceCommand::CreateTile {
                        kind: "waveform".into(),
                        placement: ids
                            .last()
                            .copied()
                            .map_or(Placement::Root, Placement::TabAfter),
                        focus: true,
                    },
                )
                .unwrap();
            let id = workspace.layout().focused().unwrap();
            let list = workspace.tiles()[&id].kind.waveform_list().unwrap();
            let mut file = workspace.to_file().unwrap();
            file.item_lists
                .get_mut(&list)
                .unwrap()
                .annotations
                .push(annotation());
            workspace = Workspace::from_file(file).unwrap();
            ids.push(id);
        }
        let annotation_id = annotation().get_id();
        let rename = || {
            TileMessage::Waveform(crate::tile_kinds::waveform::WaveformMessage::Annotation(
                AnnotationCommand::Rename {
                    annotation: annotation_id,
                    name: "Changed".into(),
                },
            ))
        };
        workspace
            .apply_tile_message(ids[0], rename(), None)
            .unwrap();
        let name = |workspace: &Workspace, id| {
            let list = workspace.tiles()[&id].kind.waveform_list().unwrap();
            workspace.item_lists()[&list].annotations[0]
                .get_name()
                .to_owned()
        };
        assert_eq!(name(&workspace, ids[0]), "Changed");
        assert_eq!(name(&workspace, ids[1]), "Rectangle 1");
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(ids[0]))
            .unwrap();
        assert!(
            !workspace
                .apply_tile_message(ids[0], rename(), None)
                .unwrap()
        );
        assert_eq!(name(&workspace, ids[1]), "Rectangle 1");
    }

    #[test]
    fn annotation_undo_targets_its_list_and_preserves_navigation_and_redo() {
        use crate::{
            Message,
            tile_kinds::waveform::{WaveformMessage, WaveformNavigation},
        };
        let mut state = crate::SystemState::new_default_config().unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            state
                .update(Message::Workspace(WorkspaceCommand::CreateTile {
                    kind: "waveform".into(),
                    placement: ids
                        .last()
                        .copied()
                        .map_or(Placement::Root, Placement::TabAfter),
                    focus: true,
                }))
                .unwrap();
            let id = state.user.workspace.layout().focused().unwrap();
            let list = state.user.workspace.tiles()[&id]
                .kind
                .waveform_list()
                .unwrap();
            let mut file = state.user.workspace.to_file().unwrap();
            file.item_lists
                .get_mut(&list)
                .unwrap()
                .annotations
                .push(annotation());
            state.user.workspace = Workspace::from_file(file).unwrap();
            ids.push(id);
        }
        let annotation_id = annotation().get_id();
        let rename = |name: &str| {
            Message::ToTile(
                ids[0],
                TileMessage::Waveform(WaveformMessage::Annotation(AnnotationCommand::Rename {
                    annotation: annotation_id,
                    name: name.into(),
                })),
            )
        };
        let name = |state: &crate::SystemState, id| {
            let list = state.user.workspace.tiles()[&id]
                .kind
                .waveform_list()
                .unwrap();
            state.user.workspace.item_lists()[&list].annotations[0]
                .get_name()
                .to_owned()
        };
        let history = state.undo_stack.len();
        state.update(rename("Changed")).unwrap();
        assert_eq!(state.undo_stack.len(), history + 1);
        state.update(rename("Changed")).unwrap();
        assert_eq!(
            state.undo_stack.len(),
            history + 1,
            "no-op creates no history"
        );
        state
            .update(Message::ToTile(
                ids[0],
                TileMessage::Waveform(WaveformMessage::Navigate(WaveformNavigation::Pan(0.2))),
            ))
            .unwrap();
        let viewport = |state: &crate::SystemState| {
            let TileKind::Waveform(tile) = &state.user.workspace.tiles()[&ids[0]].kind else {
                panic!()
            };
            ron::to_string(&tile.view.viewport).unwrap()
        };
        let navigation = viewport(&state);
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(name(&state, ids[0]), "Rectangle 1");
        assert_eq!(name(&state, ids[1]), "Rectangle 1");
        assert_eq!(state.user.workspace.layout().focused(), Some(ids[1]));
        assert_eq!(viewport(&state), navigation);
        assert_eq!(state.redo_stack.len(), 1);
        assert!(
            state
                .update(Message::ToTile(
                    ids[0],
                    TileMessage::Waveform(WaveformMessage::Annotation(
                        AnnotationCommand::MoveToGroup {
                            annotation: annotation_id,
                            group: Some("Missing".into()),
                        }
                    ))
                ))
                .is_none()
        );
        state.update(rename("Rectangle 1")).unwrap();
        assert_eq!(
            state.redo_stack.len(),
            1,
            "failed and no-op edits preserve redo"
        );
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(name(&state, ids[0]), "Changed");
        assert_eq!(name(&state, ids[1]), "Rectangle 1");
        assert_eq!(viewport(&state), navigation);
    }

    #[test]
    fn inspector_is_singleton_and_saves_its_comment_preference_without_a_document() {
        let mut state = crate::SystemState::new_default_config().unwrap();
        let open = || {
            crate::Message::Workspace(WorkspaceCommand::OpenTile {
                kind: "annotation_list".into(),
                placement: Placement::Edge(Direction::Right),
                focus: true,
            })
        };
        state.update(open()).unwrap();
        let id = state.user.workspace.layout().focused().unwrap();
        state
            .update(crate::Message::ToTile(
                id,
                TileMessage::AnnotationList(AnnotationListMessage::ShowComments(true)),
            ))
            .unwrap();
        state.update(open()).unwrap();
        assert_eq!(state.user.workspace.tiles().len(), 1);
        assert_eq!(state.user.workspace.layout().focused(), Some(id));
        let saved = state.encode_state().unwrap();
        let restored: crate::state::UserState = crate::tiles::serde::decode(&saved).unwrap();
        let TileKind::AnnotationList(tile) = &restored.workspace.tiles()[&id].kind else {
            panic!()
        };
        assert!(tile.show_comments);
        assert!(!saved.contains("show_annotation_list"));
        assert!(restored.workspace.item_lists().is_empty());
    }
}
