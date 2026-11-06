use super::{local_files, ui};
use crate::{ContributionRegistry, ShellHandle, services::AppServices};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
pub fn language_picker(cx: &App) -> Div {
    let choice = cx
        .global::<AppServices>()
        .storage
        .value("sonus.language")
        .unwrap_or("ru");
    h_flex()
        .gap_2()
        .child(format!("Language / Язык: {choice}"))
        .children(
            [("en", "English"), ("ru", "Русский")]
                .into_iter()
                .map(|(id, label)| {
                    Button::new(id)
                        .small()
                        .label(label)
                        .on_click(move |_, window, cx| {
                            if let Err(error) = cx
                                .global_mut::<AppServices>()
                                .storage
                                .set_value("sonus.language", id.into())
                            {
                                window
                                    .push_notification(Notification::error(error.to_string()), cx);
                            }
                            cx.refresh_windows();
                        })
                }),
        )
}
pub fn render(shell: ShellHandle, _window: &mut Window, cx: &App) -> impl IntoElement + use<> {
    let modes = cx.global::<ContributionRegistry>().modes();
    let recent = local_files::recent(cx);
    let open = shell.clone();
    let settings = shell.clone();
    let extensions = shell.clone();
    let demo = shell.clone();
    v_flex().size_full().id("sonus-home").overflow_y_scroll().p_6().gap_4()
 .child(div().text_2xl().child("Sonus"))
 .child(language_picker(cx))
 .child(ui::card(ui::localized(cx,"Start editing","Начать работу"),Some(ui::localized(cx,"Open a native project or choose a mode. Playback and segment editing never require an account. Extensions add optional workflows.","Откройте проект или выберите режим. Прослушивание и редактирование сегментов не требуют аккаунта. Расширения добавляют дополнительные возможности.")),cx)
  .child(h_flex().gap_2().flex_wrap()
   .child(Button::new("home-open").primary().label(ui::localized(cx,"Open project…","Открыть проект…")).on_click(move|_,window,cx|local_files::open_project(open.clone(),window,cx)))
   .child(Button::new("home-demo").label(ui::localized(cx,"Try the demo","Попробовать демо")).on_click(move|_,window,cx|{
    if let Err(error)=crate::root::open_demo(&demo,window,cx){window.push_notification(Notification::error(error.to_string()),cx);}
   }))))
 .child(ui::card(ui::localized(cx,"Modes","Режимы"),None,cx).children(modes.into_iter().map(|mode|{
  let handle=shell.clone();let id=mode.id.clone();v_flex().gap_1().child(Button::new(SharedString::from(mode.id)).label(mode.label).on_click(move|_,window,cx|handle.show_mode(&id,window,cx))).child(mode.description)
 })))
 .child(ui::card(ui::localized(cx,"Recent projects","Недавние проекты"),None,cx).children(recent.into_iter().enumerate().map(|(i,path)|{let handle=shell.clone();Button::new(("recent",i)).label(path.display().to_string()).on_click(move|_,window,cx|local_files::open_path(handle.clone(),path.clone(),window,cx))})))
 .child(ui::card(ui::localized(cx,"Recovery snapshots","Восстановление"),None,cx).children(super::recovery::entries(cx).into_iter().enumerate().map(|(i,path)|{let handle=shell.clone();Button::new(("recovery",i)).label(path.display().to_string()).on_click(move|_,window,cx|super::recovery::open(handle.clone(),path.clone(),window,cx))})))
 .child(h_flex().gap_2()
  .child(Button::new("home-settings").label(ui::localized(cx,"Settings","Настройки")).on_click(move|_,window,cx|settings.show_settings(window,cx)))
  .child(Button::new("home-extensions").label(ui::localized(cx,"Extensions","Расширения")).on_click(move|_,window,cx|extensions.show_extensions(window,cx))))
 .children(super::updates::update_notice(cx))
}
