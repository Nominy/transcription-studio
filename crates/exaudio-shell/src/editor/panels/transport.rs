use crate::editor::model::EditorModel;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable, Sizable, h_flex};
use gpui_kit::*;
pub struct TransportStrip {
    model: Entity<EditorModel>,
    _subscription: Subscription,
}
impl TransportStrip {
    pub fn new(model: Entity<EditorModel>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&model, |_, _, cx| cx.notify());
        Self {
            model,
            _subscription: subscription,
        }
    }
}
impl Render for TransportStrip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let doc = self.model.read(cx).doc();
        let undo = doc.can_undo();
        let redo = doc.can_redo();
        let read_only = doc.is_read_only();
        h_flex()
            .gap_2()
            .py_1()
            .child(
                Button::new("native-save")
                    .small()
                    .label(crate::screens::ui::localized(cx, "Save", "Сохранить"))
                    .disabled(read_only)
                    .on_click(cx.listener(|this, _, window, cx| {
                        crate::screens::local_files::save(&this.model, window, cx, None)
                    })),
            )
            .child(
                Button::new("undo")
                    .small()
                    .label(crate::screens::ui::localized(cx, "Undo", "Отменить"))
                    .disabled(!undo || read_only)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.update(cx, |m, cx| m.edit(cx, |d| d.undo()))
                    })),
            )
            .child(
                Button::new("redo")
                    .small()
                    .label(crate::screens::ui::localized(cx, "Redo", "Повторить"))
                    .disabled(!redo || read_only)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.update(cx, |m, cx| m.edit(cx, |d| d.redo()))
                    })),
            )
    }
}
