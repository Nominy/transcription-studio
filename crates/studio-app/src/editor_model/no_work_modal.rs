pub const NO_WORK_DESCRIPTION: &str = "Come back later and we'll have more work ready.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoWorkModalInput {
    pub open: bool,
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoWorkModalModel {
    pub open: bool,
    pub can_user_close: bool,
    pub title: String,
    pub description: String,
    pub button_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoWorkModalEvent {
    OpenChange { open: bool },
    ClickBackToProjects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoWorkModalEffect {
    Close,
    BackToProjects,
}

pub fn build_no_work_modal_model(input: NoWorkModalInput) -> NoWorkModalModel {
    NoWorkModalModel {
        open: input.open,
        can_user_close: false,
        title: format!(
            "Sorry, we weren't able to find a {} job for you!",
            input.mode
        ),
        description: NO_WORK_DESCRIPTION.to_string(),
        button_label: "Back to Projects".to_string(),
    }
}

pub fn reduce_no_work_modal_event(event: NoWorkModalEvent) -> Vec<NoWorkModalEffect> {
    match event {
        NoWorkModalEvent::OpenChange { open } => {
            if open {
                Vec::new()
            } else {
                vec![NoWorkModalEffect::Close]
            }
        }
        NoWorkModalEvent::ClickBackToProjects => {
            vec![NoWorkModalEffect::Close, NoWorkModalEffect::BackToProjects]
        }
    }
}
