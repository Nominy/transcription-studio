use super::*;
use crate::screens::ui::localized;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct WaveformWidget {
    owner: String,
    shell: ShellHandle,
    config: w::WaveformConfig,
    source: Option<Arc<[f32]>>,
    peaks: Arc<[f32]>,
    message: &'static str,
    cancel: Arc<AtomicBool>,
    task: Option<Task<()>>,
    _observer: Option<Subscription>,
}
impl Drop for WaveformWidget {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl WaveformWidget {
    pub(super) fn new(
        owner: String,
        shell: ShellHandle,
        config: w::WaveformConfig,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = shell.editor_model(cx);
        let observer = model
            .as_ref()
            .map(|model| cx.observe(model, |this, _, cx| this.reload(cx)));
        let mut this = Self {
            owner,
            shell,
            config,
            source: None,
            peaks: Arc::from([]),
            message: localized(cx, "Loading audio…", "Загрузка аудио…"),
            cancel: Arc::new(AtomicBool::new(false)),
            task: None,
            _observer: observer,
        };
        this.reload(cx);
        this
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.shell.editor_model(cx) else {
            self.message = localized(cx, "Project closed", "Проект закрыт");
            return;
        };
        if !crate::wasm_host::has_permission(
            &self.owner,
            exaudio_extension::Permission::AudioRead,
            cx,
        ) || !crate::wasm_host::document_associated(&self.owner, &model, cx)
        {
            self.cancel.store(true, Ordering::Release);
            self.source = None;
            self.peaks = Arc::from([]);
            self.message = localized(
                cx,
                "This extension can't read the audio",
                "Расширение не может читать аудио",
            );
            cx.notify();
            return;
        }
        if crate::wasm_host::document_handle(&model) != self.config.document_id {
            self.message = localized(cx, "Project closed", "Проект закрыт");
            return;
        }
        let model = model.read(cx);
        let Some(audio) = model.audio().lane_audio(&self.config.lane_id) else {
            return;
        };
        let Some(source) = audio.channel_data_shared(self.config.channel as usize) else {
            self.message = localized(cx, "No audio", "Нет аудио");
            return;
        };
        if self
            .source
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &source))
            && !self.cancel.load(Ordering::Acquire)
        {
            return;
        }
        let start = (self.config.start_ms as f64 * audio.sample_rate() as f64 / 1000.) as usize;
        let end = ((self.config.end_ms as f64 * audio.sample_rate() as f64 / 1000.) as usize)
            .min(source.len());
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.source = Some(source.clone());
        let cancellation = self.cancel.clone();
        if start >= end {
            self.peaks = Arc::from([]);
            self.message = localized(cx, "Nothing to show", "Нечего показать");
            cx.notify();
            return;
        }
        self.message = localized(cx, "Preparing waveform…", "Подготовка волны…");
        let work = cx.background_spawn(async move {
            let bins = (end - start).min(2048);
            let mut peaks = Vec::with_capacity(bins);
            for bin in 0..bins {
                if cancellation.load(Ordering::Acquire) {
                    return None;
                }
                let a = start + (end - start) * bin / bins;
                let b = start + (end - start) * (bin + 1) / bins;
                let mut amplitude = 0f32;
                for chunk in source[a..b].chunks(4096) {
                    if cancellation.load(Ordering::Acquire) {
                        return None;
                    }
                    for sample in chunk {
                        amplitude = amplitude.max(sample.abs());
                    }
                }
                peaks.push(amplitude.min(1.));
            }
            Some(Arc::<[f32]>::from(peaks))
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            if let Some(peaks) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.peaks = peaks;
                    this.message = "";
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }
}
impl Render for WaveformWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let allowed = self.shell.editor_model(cx).is_some_and(|model| {
            crate::wasm_host::document_associated(&self.owner, &model, cx)
                && crate::wasm_host::has_permission(
                    &self.owner,
                    exaudio_extension::Permission::AudioRead,
                    cx,
                )
                && crate::wasm_host::document_handle(&model) == self.config.document_id
        });
        if !allowed {
            self.cancel.store(true, Ordering::Release);
            self.source = None;
            self.peaks = Arc::from([]);
            return div()
                .h(px(96.))
                .child(localized(
                    cx,
                    "No audio in this project",
                    "В этом проекте нет аудио",
                ))
                .into_any_element();
        }
        if self.peaks.is_empty() {
            return div().h(px(96.)).child(self.message).into_any_element();
        }
        let peaks = self.peaks.clone();
        let color = cx.theme().primary;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let mut path = PathBuilder::stroke(px(1.));
                let count = (f32::from(bounds.size.width).ceil().max(1.) as usize).min(peaks.len());
                for column in 0..count {
                    let a = peaks.len() * column / count;
                    let b = (peaks.len() * (column + 1) / count)
                        .max(a + 1)
                        .min(peaks.len());
                    let amplitude = peaks[a..b].iter().copied().fold(0f32, f32::max);
                    let x = bounds.origin.x + bounds.size.width * (column as f32 / count as f32);
                    let middle = bounds.origin.y + bounds.size.height * 0.5;
                    let height = bounds.size.height * (amplitude * 0.5);
                    path.move_to(point(x, middle - height));
                    path.line_to(point(x, middle + height));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            },
        )
        .w_full()
        .h(px(96.))
        .into_any_element()
    }
}
