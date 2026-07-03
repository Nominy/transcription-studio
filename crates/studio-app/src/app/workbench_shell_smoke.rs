use std::path::PathBuf;

use iced::Task;

use super::Message;
use super::renderer_capture::renderer_screenshot_task;

const WORKBENCH_SHELL_SMOKE_ENV: &str = "STUDIO_SMOKE_WORKBENCH_SHELL_ON_START";

pub(super) fn workbench_shell_smoke_enabled() -> bool {
    std::env::var(WORKBENCH_SHELL_SMOKE_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn workbench_shell_renderer_screenshot_task() -> Task<Message> {
    renderer_screenshot_task(
        PathBuf::from("target")
            .join("runtime-workbench-shell-smoke")
            .join("workbench-shell-renderer.bmp"),
    )
}
