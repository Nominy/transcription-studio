# Exaudio — copy deck (EN + RU) · draft 1 for redline

Source: `ux-string-inventory` (2,045 user-facing strings). Today only 97 call sites are
localized; 253 EN-only strings sit on otherwise bilingual screens; 62 strings concatenate
both languages; 301 leak jargon, identifiers or `Debug` output. This deck replaces all of it.

## How to redline

- Edit the **EN** / **RU** columns directly. Strike a row to drop the string.
- `⟨alt⟩` marks an alternative I considered; pick one or write your own.
- `?` in the Notes column is a question for you.
- Rows marked **[gone]** are removed from the UI by the IA changes, not rewritten.
- Everything not listed here (≈ 1,300 Babel task-panel strings, format-parser errors,
  WASM host log lines) follows the *Rules* and *Glossary* below and is applied mechanically;
  it is not worth your time row by row. Babel headline strings are in §10.

---

## 1. Rules

1. **One language at a time.** Picked from the OS locale on first launch; switchable in
   Settings › General. No `EN / RU` dual strings anywhere.
2. **Sentence case** for everything except the product name and proper nouns. Buttons are
   verbs or verb phrases (`Open audio`, `Save`), never nouns (`Export…` → `Export…` is fine,
   `Document ID` → gone).
3. **No implementation words** in the UI: lane, cue, diagnostics, provenance, generation,
   transaction, provider, backend, manifest, WASM, DLL, sandbox, coordinator, revision, native,
   runtime, contribution, identity, mutation, store. See glossary for replacements.
4. **No identifiers, hashes, paths or `Debug` output** in sentences. Paths appear only as
   secondary text under a human name. Errors: one plain sentence, optional *Details*
   disclosure with the technical text.
5. **Numbers and units**: `1 track`, `24 segments`, `00:15.45`, `2.6 GB`. Plurals are real
   plurals (RU: 1 сегмент / 2 сегмента / 5 сегментов), never `segment(s)`.
6. **RU register**: «вы» lowercase, imperative buttons (`Открыть`, `Сохранить`), no
   calques (`транзакция`, `провайдер`, `нативный`, `сабаккаунт`).
7. **Product name** is `Exaudio` in both languages; never declined in RU (`в Exaudio`).
8. **Tone**: short, calm, specific. Never "oops", never "!" in errors, never apologies.

---

## 2. Glossary

| Concept (today) | EN | RU | Notes |
|---|---|---|---|
| lane | **track** | **дорожка** | |
| segment / region / row / cue | **segment** | **сегмент** | ⟨alt: *line / строка*⟩ — "segment" is familiar from subtitle tools; "line" is plainer. ? |
| speaker | speaker | спикер | ⟨alt RU: *говорящий*⟩ ? |
| transcript | transcript | расшифровка | |
| project / document / native project | **project** | **проект** | "document" never shown |
| mode (Local, Babel…) | **workspace** | **рабочее пространство** (short: *пространство*) | ⟨alt: *source / источник*⟩ ? |
| extension / marketplace / package | **extension** / **catalog** | **расширение** / **каталог** | |
| diagnostics / lint / issues | **checks** | **проверки** | column: *Checks* / *Проверки* |
| diagnostic severity error / warning / hint | problem / warning / tip | проблема / предупреждение / подсказка | |
| quick fix | fix | исправить | |
| provider / backend / inference location | **runs on** · This computer / Remote server | **где выполняется** · Этот компьютер / Удалённый сервер | "This PC" → "This computer" |
| Gold (the AI extension) | Gold (extension name only) | Gold | never in button labels |
| Word timestamps | **Align words** | **Выровнять слова** | |
| Gold Draft | **Draft with AI** | **Черновик с ИИ** | dialog title: *AI draft* / *Черновик ИИ* |
| Transcribe segment | **Fill this segment** | **Заполнить сегмент** | |
| Denoise / enhanced audio | **Denoise** / clean audio | **Убрать шум** / очищенный звук | ⟨alt RU: *Шумоподавление*⟩ |
| polish / refinement (OpenRouter) | **Polish** | **Доработка текста** | |
| recovery snapshot | **unsaved work** | **несохранённая работа** | |
| playhead | playhead | курсор воспроизведения | |
| loop | loop | повтор | |
| annotation (Babel) | annotation | аннотация | Babel only |
| relay / sub-account (Babel) | relay / sub-account | ретранслятор / под-аккаунт | Babel only; ⟨alt RU: *реле* / *под-аккаунт*⟩ ? |
| claim / submit (Babel) | take / submit | взять / отправить | Babel only |
| timeline | timeline | таймлайн | ⟨alt RU: *шкала времени*⟩ ? |
| waveform | waveform | волна | |
| zoom | zoom | масштаб | |

---

## 3. Product-level

| Where | Current | EN | RU | Notes |
|---|---|---|---|---|
| Window title / tray | `Sonus` | `Exaudio` | `Exaudio` | |
| Tray | `Open Sonus` / `Exit` | `Open Exaudio` / `Quit` | `Открыть Exaudio` / `Выйти` | |
| Update notice | `Sonus {version} is available` | `Exaudio {version} is ready to install` | `Доступна версия Exaudio {version}` | |
| Update buttons | `Install` / `Skip` | `Update` / `Not now` | `Обновить` / `Не сейчас` | |
| Update failure | `Skipped version could not be saved: {error}` | `Couldn't remember the skipped version.` + Details | `Не удалось запомнить пропущенную версию.` + Подробности | |
| Tagline (site/README) | `Audio meets text.` | `From audio, every word.` ⟨alt: *Hear it. Write it down.*⟩ | `Из звука — в слова.` ⟨alt: *Услышать. Записать.*⟩ | ? |

---

## 4. Start screen and mode launch windows

Start is the list of installed modes ("server list"). Each tile: label, one-line description,
and an account chip only when its provider type has more than one account. Last tile:
*Get more*. Footer: Settings · Extensions. Clicking a tile opens the mode's launch window; if the
mode needs an account that isn't set up yet, the sign-in page appears first, as part of
opening the mode. Providers never appear by name outside Settings › Advanced › Accounts.

### 4.1 Start

| Element | Current | EN | RU | Notes |
| --- | --- | --- | --- | --- |
| Heading | `Sonus` | *(wordmark, no text)* | — |  |
| Language picker | `Language / Язык: ru [English][Русский]` | **[gone]** → Settings › General |  | first run uses OS locale |
| Tile | `Local workspace` / `Open audio and edit projects without an account.` | `Local` / `Open audio and projects on this computer` | `Локально` / `Аудио и проекты на этом компьютере` |  |
| Tile | `Babel` / `Direct Babel account tasks and review` | `Babel` / `Transcription tasks` | `Babel` / `Задачи транскрипции` | needs Babel account |
| Tile | *(new)* | `Babel error review` / `Error review tasks` | `Babel: ревью ошибок` / `Задачи ревью ошибок` | needs Babel account |
| Tile | `Babel JSON file` / `Offline Babel file editing; no account or remote mutations` | `Babel file` / `Edit an exported task offline` | `Файл Babel` / `Редактировать экспортированную задачу офлайн` | opens file picker directly |
| Tile | `Babel Relay` | **[gone]** — sub-account code is a sign-in method of the Babel account |  |  |
| Tile | — | `Get more` / `Extensions from the catalog` | `Ещё` / `Расширения из каталога` | → Extensions page |
| Account chip | — | `{label}` (tooltip `Switch account`) | `{label}` (`Сменить аккаунт`) | only when >1 account |
| Footer | `Settings` · `Extensions` | `Settings` · `Extensions` | `Настройки` · `Расширения` |  |
| Mode page header | `Home` · `Settings` · `Extensions` | `Start` · `Settings` · `Extensions` | `Начало` · `Настройки` · `Расширения` | header for launch windows |

### 4.2 Sign-in step (shell page wrapping the provider's setup view)

| Element | EN | RU | Notes |
|---|---|---|---|
| Page title | `Sign in to {mode}` | `Вход: {mode}` | e.g. "Sign in to Babel" |
| Cancel | `Cancel` (page) / `Later` (after Extensions install) | `Отмена` / `Позже` | |
| Chooser title | `Which account?` | `Какой аккаунт?` | only when >1 account and none chosen |
| Chooser row | `{label}` · `{status detail}` | | |
| Chooser action | `Add another account` | `Добавить ещё аккаунт` | `multiple` types only |
| Status detail | `Signed in` / `Sign in again` / `{error}` | `Выполнен вход` / `Войдите снова` / `{error}` | |

### 4.3 Local launch window

| Element | Current | EN | RU | Notes |
| --- | --- | --- | --- | --- |
| Card title | `Start editing` | `Start` | `Начать` |  |
| Card text | `Open a native project or choose a mode. …` | `Open a recording and start editing.` | `Откройте запись и начните редактировать.` |  |
| Primary | `Open audio` | `Open audio…` | `Открыть аудио…` | WAV · MP3 · FLAC · OGG · M4A · MP4 · WebM · MKV as subtext |
| Secondary | `Open project…` | `Open project…` | `Открыть проект…` |  |
| Secondary | `Try the demo` | `Try the demo` | `Попробовать демо` |  |
| Section | `Recent projects` | `Recent` | `Недавние` | rows: project name; path as tooltip; hidden when empty |
| Section | `Recovery snapshots` | `Unsaved work` | `Несохранённая работа` | rows: project name; hidden when empty |
| Row action | *(click path)* | `Restore` | `Восстановить` |  |
| Import progress | `{done}/{total} — {file}` | `Opening {done} of {total}: {file}` | `Открытие {done} из {total}: {file}` |  |
| Import cancel | `Cancel` | `Cancel` | `Отмена` |  |
| New project name (1 file) | `Untitled` | *(file name without extension)* | *(то же)* |  |
| New project name (N files) | `New transcript` | `New project` | `Новый проект` |  |
| Default speaker | `Speaker {n}` | `Speaker {n}` | `Спикер {n}` |  |
| Default track | `Audio` | `Track {n}` | `Дорожка {n}` |  |

### File dialogs (OS prompts)

| Current | EN | RU |
|---|---|---|
| `Open project` | `Open an Exaudio project` | `Открыть проект Exaudio` |
| `Save native project here` | `Choose where to save the project` | `Выберите, куда сохранить проект` |
| `Save native copy here` | `Choose where to save a copy` | `Выберите, куда сохранить копию` |
| `Unsaved changes` / `Save this document before leaving?` / `Save` `Discard` `Cancel` | `Unsaved changes` / `Save changes to “{project}”?` / `Save` `Don't save` `Cancel` | `Несохранённые изменения` / `Сохранить изменения в «{project}»?` / `Сохранить` `Не сохранять` `Отмена` |

---

## 5. Editor

### 5.1 Toolbar (left → right)

Proposed order: `← Start` · transport (play, speed) · zoom · **Save / Undo / Redo** (icons +
dirty dot) · *spacer* · **AI** menu · **Export…** · **⋯** overflow. The transcript-area
`Save / Undo / Redo` buttons move up here. `Document ID` is removed.

| Element | Current | EN | RU | Notes |
|---|---|---|---|---|
| Back | `Projects` / tooltip `Back to projects` | `Start` / `Back to Start` | `Начало` / `На начальный экран` | |
| Play/Pause tooltips | `Play` / `Pause` | `Play` / `Pause` | `Воспроизвести` / `Пауза` | + shortcut |
| Speed tooltip | `Playback speed` | `Speed` | `Скорость` | |
| Slower / Faster | same | `Slower` / `Faster` | `Медленнее` / `Быстрее` | |
| Zoom tooltips | `Zoom out (Ctrl+wheel)` / `Zoom in (Ctrl+wheel)` | `Zoom out` / `Zoom in` (shortcut appended) | `Уменьшить` / `Увеличить` | |
| Save state badge | `Saved` / `Unsaved` / `Saving…` / `Save failed` | `Saved` / `Unsaved changes` / `Saving…` / `Not saved` | `Сохранено` / `Есть изменения` / `Сохранение…` / `Не сохранено` | badge → dot on Save button + tooltip |
| Read-only tag | `Read-only` | `Read only` | `Только чтение` | |
| Save / Undo / Redo | same | `Save` / `Undo` / `Redo` | `Сохранить` / `Отменить` / `Повторить` | icons, labels in tooltips |
| Document ID button + toast | `Document ID` / `Document ID copied.` | **[gone]** (moves to ⋯ › *Copy project ID*) | `Скопировать ID проекта` | Babel needs it; nobody else |
| AI menu | *(scattered toolbar buttons)* | `AI` | `ИИ` | groups the four Gold actions |
| AI › | `Word timestamps` | `Align words` | `Выровнять слова` | |
| AI › | `Gold Draft` | `Draft with AI…` | `Черновик с ИИ…` | |
| AI › | `Transcribe segment` | `Fill this segment` | `Заполнить сегмент` | shortcut shown |
| AI › | `Denoise` | `Denoise` | `Убрать шум` | |
| AI › | `AI settings` | `AI settings…` | `Настройки ИИ…` | |
| Export | `Export…` | `Export…` | `Экспорт…` | |
| ⋯ menu | `Commands` | `⋯` (tooltip `More`) | `Ещё` | |
| ⋯ › | `Import transcript…` | `Import transcript…` | `Импортировать текст…` | |
| ⋯ › | `Home` / `Settings` / `Extensions` | `Start` / `Settings…` / `Extensions…` | `Начало` / `Настройки…` / `Расширения…` | |
| ⋯ › | `Keyboard shortcuts` (icon) | `Keyboard shortcuts…` | `Сочетания клавиш…` | |
| ⋯ › | `Start subaccount relay` / `Stop …` | *(Babel only)* `Start sub-account relay` / `Stop sub-account relay` | `Запустить ретранслятор` / `Остановить ретранслятор` | only when Babel extension active |

### 5.2 Timeline

| Element | Current | EN | RU |
|---|---|---|---|
| Loading | `Loading audio…` | `Loading audio…` | `Загрузка аудио…` |
| Lane header tooltips | `Expand lane` / `Collapse lane` / `Mute lane` / `Unmute lane` | `Expand track` / `Collapse track` / `Mute` / `Unmute` | `Развернуть дорожку` / `Свернуть дорожку` / `Без звука` / `Включить звук` |
| Zoom readout | `20.0×` | **[gone]** — slider only, tooltip `Waveform height` | `Высота волны` |
| Loop chip | `Loop` | `Loop` | `Повтор` |
| Selection length chip | `{s} s` | `{s} s` | `{s} с` |
| Edit annotation tooltip | `Edit annotation` | *(Babel)* `Edit annotation` | `Изменить аннотацию` |

### 5.3 Transcript table

| Element | Current | EN | RU | Notes |
|---|---|---|---|---|
| Columns | `#` `Speaker` `Start` `End` `Reviewer` `Text` `Issues` | `#` `Speaker` `Start` `End` `Reviewer` `Text` `Checks` | `№` `Спикер` `Начало` `Конец` `Ревьюер` `Текст` `Проверки` | Reviewer only in Babel |
| Empty state | `No segments yet` / `Click and drag on the waveform above to create a region, then add text.` | `No segments yet` / `Drag across the waveform to create a segment, then type what was said.` | `Пока нет сегментов` / `Выделите участок на волне, чтобы создать сегмент, и введите текст.` | |
| Text placeholder | `What was said…` | `What was said…` | `Что было сказано…` | |
| Annotation placeholder | `Annotation…` | `Annotation…` | `Аннотация…` | Babel |
| Time placeholder | `mm:ss.cc` | `mm:ss.cc` | `мм:сс.сс` | |
| Row menu | `Edit` / `Add Segment Above` / `Add Segment Below` / `Merge With Above` / `Merge With Below` / `Delete` | `Edit` / `Add segment above` / `Add segment below` / `Merge with previous` / `Merge with next` / `Delete` | `Изменить` / `Добавить сегмент выше` / `Добавить сегмент ниже` / `Объединить с предыдущим` / `Объединить со следующим` / `Удалить` | |
| Quick fix | `Quick fix: {title}` | `Fix: {title}` | `Исправить: {title}` | |
| Review chip tooltip | `Mark word reviewed` | *(Babel)* `Mark as reviewed` | `Отметить как проверенное` | |
| Diff badges | `New` `Added` `Deleted` `No changes` `1 change` `{n} changes` `Unchanged` `Label changed` `Subcategory changed` `Intensity changed` `Removed` `No content` | same (Babel compare view) | `Новый` `Добавлен` `Удалён` `Без изменений` `1 изменение` `{n} изменений` `Без изменений` `Изменена метка` `Изменена подкатегория` `Изменена интенсивность` `Удалено` `Пусто` | |

### 5.4 Find bar

| Current | EN | RU |
|---|---|---|
| `Find in transcript` | `Find in transcript` | `Поиск в расшифровке` |
| `Type to find` / `No matches` / `{i} of {n}` | `Type to search` / `No matches` / `{i} of {n}` | `Введите текст` / `Ничего не найдено` / `{i} из {n}` |
| `Previous match` / `Next match` / `Close` | same | `Предыдущее` / `Следующее` / `Закрыть` |

### 5.5 Status bar

| Current | EN | RU |
|---|---|---|
| `Segment {i} of {n}` / `{n} segments` | `Segment {i} of {n}` / `{n} segments` | `Сегмент {i} из {n}` / `{n} сегментов` |
| `Zoom 1.00× · 6.2s visible` | **[gone]** (zoom is on the slider) | |
| `Loading audio {i}/{n}` | `Loading audio {i} of {n}` | `Загрузка аудио {i} из {n}` |
| `1 lane without audio` / `{n} lanes without audio` | `1 track has no audio` / `{n} tracks have no audio` | `1 дорожка без аудио` / `{n} дорожек без аудио` |
| `Saved {time}` | `Saved {time}` | `Сохранено {time}` |
| `Save failed: {message}` | `Not saved — {message}` | `Не сохранено — {message}` |
| `Audio playback unavailable: {error}` | `Can't play audio on this device.` + Details | `Не удаётся воспроизвести звук на этом устройстве.` + Подробности |
| `{lane} audio: {error}` | `Couldn't load audio for {track}.` + Details | `Не удалось загрузить аудио для «{track}».` + Подробности |
| `Saved native copy to {path}` | `Copy saved to {folder}` | `Копия сохранена в {folder}` |
| `Native copy failed: {error}` | `Couldn't save a copy.` + Details | `Не удалось сохранить копию.` + Подробности |
| `Recovery not saved: {error}` / `Recovery index not saved: {error}` | `Couldn't save a backup of unsaved work.` + Details | `Не удалось сохранить резервную копию.` + Подробности |

### 5.6 Segment dialog (create region)

| Current | EN | RU |
|---|---|---|
| title `Segment` | `New segment` | `Новый сегмент` |
| placeholder `Segment text` | `What was said…` | `Что было сказано…` |
| track dropdown shows lane **id** | *(shows track name)* | |
| `Cancel` / `Apply` | `Cancel` / `Add` | `Отмена` / `Добавить` |

### 5.7 Keyboard shortcuts dialog

| Current | EN | RU |
| --- | --- | --- |
| `Keyboard Shortcuts` | `Keyboard shortcuts` | `Сочетания клавиш` |
| `Shortcuts use physical key positions, so the same keys work across keyboard layouts.` | `[gone]` | `[gone]` |
| `Press the new key combination for the highlighted action, or Cancel.` | `Press the new keys.` | `Нажмите новое сочетание.` |
| `Press new key combination...` | `Press keys…` | `Нажмите клавиши…` |
| badges `custom` / `conflict` / `fixed` | `changed` / `conflict` / `fixed` | `изменено` / `конфликт` / `фиксировано` |
| `Change` / `Reset` / `Reset All` / `Close` / `Cancel` | `Change` / `Reset` / `Reset all` / `Close` / `Cancel` | `Изменить` / `Сбросить` / `Сбросить все` / `Закрыть` / `Отмена` |
| `Conflicting shortcuts` | `These shortcuts conflict` | `Эти сочетания конфликтуют` |
| `Hotkey updated.` / `Hotkey reset to default.` / `All hotkeys reset to defaults.` / `Hotkey change canceled.` | `Shortcut changed` / `Shortcut reset` / `All shortcuts reset` / `Cancelled` | `Сочетание изменено` / `Сочетание сброшено` / `Все сочетания сброшены` / `Отменено` |
| `Hotkey updated. {chord} is also used by: {actions}.` | `Shortcut changed. {chord} is also used for {actions}.` | `Сочетание изменено. {chord} также используется для: {actions}.` |
| `Hotkey preference could not be saved: {error}` | `Couldn't save the shortcut.` + Details | `Не удалось сохранить сочетание.` + Подробности |

---

## 6. Checks (lint extension messages)

| Current | EN | RU |
| --- | --- | --- |
| `Leading horizontal whitespace` | `Starts with a space` | `Начинается с пробела` |
| `Trailing horizontal whitespace` | `Ends with a space` | `Заканчивается пробелом` |
| `Repeated horizontal whitespace` | `Double spaces` | `Двойные пробелы` |
| fix `Normalize this whitespace` | `Remove extra spaces` | `Убрать лишние пробелы` |
| `Segment has no transcript text` | `Empty segment` | `Пустой сегмент` |
| `Timing must be finite, nonnegative and ordered` | `End time is before start time` | `Конец раньше начала` |
| `Segment overlaps another segment in the same lane` | `Overlaps the next segment on this track` | `Пересекается со следующим сегментом на дорожке` |
| Extension name `Basic transcript checks` | `Basic checks` | `Базовые проверки` |
| Extension description | `Optional Local checks for ASCII horizontal whitespace, empty transcripts, invalid timing and same-lane overlaps. No language or punctuation conventions are enforced.` → `Flags empty segments, stray spaces, bad timing and overlaps.` | `Отмечает пустые сегменты, лишние пробелы, ошибки тайминга и пересечения.` |

---

## 7. Import / Export

| Current | EN | RU |
|---|---|---|
| dialog `Export transcript` / `Import transcript` | `Export as…` / `Import from…` | `Экспортировать как…` / `Импортировать из…` |
| format `SubRip subtitles` / `WebVTT subtitles` / `Plain text (untimed import)` | `SubRip (.srt)` / `WebVTT (.vtt)` / `Plain text (.txt)` | same, with subtext for TXT import: `Text only, no timing` / `Только текст, без тайминга` |
| `Open an audio project first, then choose a format.` | `Open a project first.` | `Сначала откройте проект.` |
| `No format extension is enabled. Open Extensions to enable one.` | `No export formats are enabled. Turn one on in Extensions.` | `Форматы экспорта отключены. Включите их в Расширениях.` |
| `The format extension was disabled.` | `That format was just turned off.` | `Этот формат только что отключили.` |
| `Exported {path}` | `Exported to {file}` | `Экспортировано в {file}` |
| `Export path has no directory.` | `Choose a folder to export to.` | `Выберите папку для экспорта.` |
| `Select a segment on the destination lane first.` | `Select a segment on the target track first.` | `Сначала выберите сегмент на нужной дорожке.` |
| `Transcript file exceeds 32 MiB.` | `This file is too large to import (over 32 MB).` | `Файл слишком большой для импорта (больше 32 МБ).` |
| `Plain text has no timestamps. Select an existing segment to place it without inventing timing.` | `Plain text has no timing. Select the segment to put it in.` | `В обычном тексте нет тайминга. Выберите сегмент, куда его вставить.` |
| `Replace the selected segment's text? Its existing timing is unchanged.` | `Replace the text of the selected segment?` | `Заменить текст выбранного сегмента?` |
| `{n} cues will be appended to this lane. Existing segments stay unchanged; source speaker labels are kept in the text.` | `Add {n} subtitles to “{track}”? Existing segments are kept. Speaker names from the file stay in the text.` | `Добавить {n} субтитров на дорожку «{track}»? Существующие сегменты сохранятся. Имена спикеров из файла останутся в тексте.` |
| title `Confirm import` / button `Import` | `Import` / `Import` | `Импорт` / `Импортировать` |
| `The active document changed. Start the import again.` | `The project changed meanwhile. Start the import again.` | `Проект изменился. Начните импорт заново.` |
| parser errors (`missing cue timing line`, …) | shown as `Couldn't read the file: line {n} — {reason}` + Details; reasons stay EN technical | `Не удалось прочитать файл: строка {n} — {reason}` |
| default export name `transcript` | `transcript` | `расшифровка` |

---

## 8. Extensions page

Structure: **list with toggles** (name, one line, switch) → details on select → an
**Advanced** disclosure at the bottom holding URL install, developer folder, catalog
source, logs, rollback.

### 8.1 Page

| Current | EN | RU |
| --- | --- | --- |
| `Extensions / Marketplace` | `Extensions` | `Расширения` |
| `Bundled extensions work offline. Marketplace packages are sandboxed WASM; native DLL loading is not supported.` | `[gone]` | `[gone]` |
| `Search extensions` | `Search extensions` | `Поиск расширений` |
| `Refresh` / `Use cache / offline` / `Retry activation` / `Review rollback` | `Refresh catalog` / *(Advanced)* `Work offline` / `Retry` / `Restore previous set…` | `Обновить каталог` / `Без сети` / `Повторить` / `Вернуть предыдущий набор…` |
| `Catalog state` + `Unavailable · bundled extensions remain available` / `Offline · validated cache only` / `Cached · …` / `Online · validated index` | `Catalog: unavailable` / `offline` / `cached` / `up to date` | `Каталог: недоступен` / `без сети` / `из кэша` / `обновлён` |
| row `{name} · {version} · bundled/installed/developer/catalog` | `{name}` + muted `{version} · Built in / Installed / Developer / Available` | `… · Встроено / Установлено / Разработка / Доступно` |
| `No extensions registered and no validated catalog available. Check the index source and refresh.` | `No extensions available. Check the catalog source under Advanced.` | `Расширения недоступны. Проверьте источник каталога в разделе «Дополнительно».` |
| `No matching extensions. Clear or change the search query.` | `Nothing matches your search.` | `По запросу ничего не найдено.` |
| `Operation failed` | `Something went wrong` + sentence + Details | `Что-то пошло не так` + предложение + Подробности |
| `Cancel pending work` | `Cancel` | `Отмена` |

### 8.2 Details

| Current | EN | RU |
|---|---|---|
| `ID` | **[gone]** (Advanced › Details) | |
| `Source` + `Bundled · native` / `Installed · reviewed index` / `Developer package · {path}` / `Developer folder · {path}` / `Catalog` | `From` + `Built in` / `Catalog` / `Developer file` / `Developer folder` (path as subtext) | `Откуда` + `Встроено` / `Каталог` / `Файл разработчика` / `Папка разработчика` |
| `State` + `Enabled` / `Not active` | `Status` + `On` / `Off` | `Состояние` + `Включено` / `Выключено` |
| `Version` / `License` | same | `Версия` / `Лицензия` |
| `Required host API` | **[gone]** (Advanced) | |
| `Required by active mode` | `Needed by the open workspace` | `Нужно открытому пространству` |
| `Document scope` + `All documents · explicit session approval` / `Only modes requiring this extension` | `Can access` + `All projects (this session)` / `Its own workspace only` | `Доступ` + `Все проекты (в этой сессии)` / `Только своё пространство` |
| `Author / repo` + `Bundled with this application; no catalog provenance declared` / `Not declared in a validated catalog` / `Catalog author (local source is unreviewed)` | `Author` + `Exaudio` / `Unknown` / `{author} (not verified)` | `Автор` + `Exaudio` / `Неизвестно` / `{author} (не проверено)` |
| `Original release` / `Open original GitHub release` | `Release page` / `Open on GitHub` | `Страница релиза` / `Открыть на GitHub` |
| `Incompatible version` / `Compatible` | `Not compatible with this version of Exaudio` / *(nothing when compatible)* | `Несовместимо с этой версией Exaudio` |
| `No compatible reviewed release is available. Refresh the index or inspect its withdrawal/API requirements.` | `No compatible version in the catalog.` | `В каталоге нет совместимой версии.` |
| `Permissions` + rows | `This extension can:` + rows below | `Это расширение может:` |
| `Read document / Чтение документа` | `read your transcript` | `читать расшифровку` |
| `Edit document / …` | `edit your transcript` | `изменять расшифровку` |
| `Read audio samples / …` | `read the audio` | `читать аудио` |
| `Control playback and seek / …` | `control playback` | `управлять воспроизведением` |
| `Change editor selection / …` | `change the selection` | `менять выделение` |
| `Request file picker / …` | `ask you to pick files` | `запрашивать выбор файлов` |
| `Private extension storage / …` | `keep its own settings` | `хранить свои настройки` |
| `Private extension secrets / …` | `store keys and passwords` | `хранить ключи и пароли` |
| `HTTPS network access / Сетевой доступ: {host} (exact host / точный адрес)` | `connect to {host}` | `подключаться к {host}` |
| `Permissions: none / …` | `This extension needs no permissions.` | `Дополнению не нужны разрешения.` |
| `Dependencies` + `None` / `Dependency / Зависимость: {id} {version}` | `Also needs` + `nothing` / `{name} {version}` | `Также нужно` + `ничего` / `{name} {version}` |
| `Review install` / `Review update` | `Install…` / `Update…` | `Установить…` / `Обновить…` |
| `Disable` / `Enable` / `Uninstall` | `Turn off` / `Turn on` / `Remove` | `Выключить` / `Включить` / `Удалить` |
| `Review / reload folder` / `Stop development extension` | `Reload` / `Stop` | `Перезагрузить` / `Остановить` |
| `Bundled files are read-only and cannot be uninstalled from this page.` | `Built-in extensions can be turned off but not removed.` | `Встроенные расширения можно выключить, но не удалить.` |
| `Close the active mode/document before disabling or uninstalling.` | `Close the open project first.` | `Сначала закройте открытый проект.` |

### 8.3 Consent dialog (replaces the preview card)

| Current | EN | RU |
|---|---|---|
| `Review changes before approval` | `Install {name}?` / `Update {name}?` / `Turn off {name}?` / `Remove {name}?` | `Установить {name}?` / `Обновить {name}?` / `Выключить {name}?` / `Удалить {name}?` |
| `Installation generation / Поколение установки: 0 · selected / выбрано: {id}` | **[gone]** | |
| `UNREVIEWED DEVELOPER SOURCE / …: {path}\nSHA256: {hash}\n{bytes} bytes` | banner `This file isn't from the catalog. Only install extensions you trust.` + Details (path, size, SHA-256) | `Этот файл не из каталога. Устанавливайте только расширения, которым доверяете.` |
| `Install { id: "…", version: Version { major: 0, … } }` (Debug) | `{name} {version}` | |
| `NEW GRANTS / …: {id}` + permission rows | `It will be able to:` + rows from 8.2 | `Оно сможет:` |
| `{id} ← sonus.marketplace-install` | **[gone]** | |
| `BLOCKED / ЗАПРЕЩЕНО: {error}` | `Can't continue: {sentence}` | `Нельзя продолжить: {sentence}` |
| `UNREVIEWED WATCHED FOLDER / …` + `Approval trusts future code changes in this folder…` | `Developer folder. Changes you make in it are loaded automatically.` | `Папка разработчика. Изменения в ней подхватываются автоматически.` |
| `Runtime contributions, publications and subscriptions are removed on disable/uninstall. Retained rollback packages are not deleted. / …` | *(turn off)* `Its panels and commands go away.` | `Его панели и команды исчезнут.` |
| `Restore previous COMPLETE dependency set / …, generation {n}` | `Go back to the previous set of extensions?` | `Вернуть предыдущий набор расширений?` |
| `Approval grants exactly the permissions shown above to these versions. Dependencies are installed as one set. Cancel leaves the current installation unchanged.` | **[gone]** | |
| `All documents: not allowed (click to opt in)` / `All documents: allowed for this session` + long paragraph | checkbox `Allow access to all projects until I quit Exaudio` | `Разрешить доступ ко всем проектам до выхода из Exaudio` |
| `Approve and apply` / `Cancel` | `Install` / `Update` / `Turn off` / `Remove` + `Cancel` | `Установить` / `Обновить` / `Выключить` / `Удалить` + `Отмена` |
| blockers: `This identity is installed/bundled; use a distinct developer ID or uninstall the managed package first. / …` | `An extension with this ID is already installed.` | `Дополнение с таким ID уже установлено.` |
| `Enable dependency first / …: {id} {version}` | `Turn on {name} first.` | `Сначала включите {name}.` |
| `Required by the active mode; close its document first. / …` | `The open project needs it. Close the project first.` | `Оно нужно открытому проекту. Сначала закройте проект.` |
| `{a} requires {b} {v}; disable the dependent extension first.` | `{a} needs it. Turn off {a} first.` | `Оно нужно для «{a}». Сначала выключите «{a}».` |

### 8.4 Status line (replaces 20 bilingual strings)

| Current | EN | RU |
|---|---|---|
| `Loading validated index / …` | `Refreshing catalog…` | `Обновление каталога…` |
| `Catalog refresh finished / …` | `Catalog up to date` | `Каталог обновлён` |
| `Restoring installed extensions / …` · `Restoring {i}/{n}: {owner}` · `Restored {n}/{n} managed extensions / …` | `Starting extensions…` · `Starting {name} ({i} of {n})…` · *(nothing on success)* | `Запуск расширений…` · `Запуск {name} ({i} из {n})…` |
| `Inspecting local source / …` · `Local package ready for review / …` · `Developer folder ready for review / …` | `Checking file…` · *(opens consent dialog)* | `Проверка файла…` |
| `Preparing transaction / …` · `Activating runtime and contributions / …` · `Activating / …: {owner}` · `Extension transaction completed and runtime activated / …` | `Installing…` · `Starting {name}…` · `{name} installed` | `Установка…` · `Запуск {name}…` · `{name} установлено` |
| `Stopping / Остановка: network and staging cancel; …` | `Cancelling…` | `Отмена…` |
| `Inspection cancelled; no changes applied / …` | `Cancelled. Nothing changed.` | `Отменено. Ничего не изменилось.` |
| `Bundled extension state changed / …` | `{name} turned on` / `{name} turned off` | `{name} включено` / `{name} выключено` |
| `Developer watcher started; …` / `Developer watcher stopped / …` / `Stopping developer extension / …` / `Extension stopped / …` | `Watching {folder}` / `Stopped watching` / `Stopping…` / `Stopped` | `Слежу за {folder}` / `Слежение остановлено` / `Остановка…` / `Остановлено` |
| `Recovering prior installation / …: {failure}` · `Prior installation restored / …` · `RECOVERY ERROR / …` | `Something went wrong; restoring the previous state…` · `Previous state restored` · `Couldn't restore the previous state. Restart Exaudio.` + Details | `Произошла ошибка; возвращаю предыдущее состояние…` · `Предыдущее состояние восстановлено` · `Не удалось восстановить предыдущее состояние. Перезапустите Exaudio.` |
| `{:?} · {} · {}/{total} {unit} · {:.1}s` (Debug stage) | `Downloading {name} — {done} of {total} MB` / `Verifying…` / `Unpacking…` | `Загрузка {name} — {done} из {total} МБ` / `Проверка…` / `Распаковка…` |
| `{url}: {error}. Bundled extensions remain available. / …` | `Couldn't reach the catalog.` + Details | `Не удалось открыть каталог.` |
| `Installation state: {error}. Do not modify extensions until the store is readable.` | `Extension settings couldn't be read. Restart Exaudio before changing extensions.` + Details | `Не удалось прочитать настройки расширений. Перезапустите Exaudio.` |
| `Installed extension validation failed: {error}. Enable missing dependencies or disable the affected package, then retry activation.` / `Startup activation failed: …` | `Some extensions couldn't start.` + per-extension reason + `Retry` | `Некоторые расширения не запустились.` |
| `URL is not an exact reviewed release URL in the current index. …` | `This link isn't in the catalog. To install anyway, download the file and use “Install from file”.` | `Этой ссылки нет в каталоге. Чтобы установить всё равно, скачайте файл и используйте «Установить из файла».` |
| `Choose a .exaudio package. / …` | `Choose an .exaudio file.` | `Выберите файл .exaudio.` |

? Package extension: rename `.exaudio` → `.exaudio`? (affects catalog + packer)

### 8.5 Advanced disclosure

| Current | EN | RU |
|---|---|---|
| card `Install package / development` | `Advanced` | `Дополнительно` |
| `Review URL` + placeholder | `Install from link…` + `https://…/*.exaudio` | `Установить по ссылке…` |
| `URLs must exactly match a release in the reviewed index. Unindexed files require separate developer-source approval.` | `Only links listed in the catalog are accepted.` | `Принимаются только ссылки из каталога.` |
| `Open .exaudio…` / `Load developer folder…` | `Install from file…` / `Open developer folder…` | `Установить из файла…` / `Открыть папку разработчика…` |
| `Developer folders are explicitly trusted, watched with debounce, and never loaded as native code. …` | `Changes in a developer folder are reloaded automatically. If a reload fails, the last working version keeps running.` | `Изменения в папке разработчика подхватываются автоматически. Если перезагрузка не удалась, продолжает работать последняя рабочая версия.` |
| card `Trusted index source` + paragraph | `Catalog source` + `Extensions are listed and verified by this catalog. Change it only if you know who runs the new one.` | `Источник каталога` + `Этот каталог перечисляет и проверяет расширения. Меняйте его, только если знаете, кто ведёт новый.` |
| `Default: {url}` / `Trust source and refresh` | `Default` link / `Use this source` | `По умолчанию` / `Использовать этот источник` |
| card `Extension logs` + `{SystemTime:?} {msg}` | `Log` + `HH:MM:SS  {extension}  {message}` | `Журнал` |

---

## 9. Settings

Sidebar: **General · AI · Updates** (+ one entry per active platform extension, e.g. **Babel**).
General ends with an **Advanced** disclosure holding *Accounts*: one row per signed-in
account (`{label}` · `Signed in` / `Sign in again` · `Sign out`), plus `Add another account`
for providers that allow several. The provider type's name (`Babel account`) is the group
heading. RU: `Аккаунты`, `Выйти`, `Добавить ещё аккаунт`.

### 9.1 General

| Current | EN | RU |
|---|---|---|
| — | section `Language` + `English` / `Русский` | `Язык` |
| `Appearance` / `Theme and the transcript layout used when a task opens.` | `Appearance` / *(no subtitle)* | `Оформление` |
| `Theme` + `Light` / `Dark` | `Theme` + `Light` / `Dark` / `System` | `Тема` + `Светлая` / `Тёмная` / `Как в системе` |
| `Interface size (Ctrl + / Ctrl − / Ctrl 0)` | `Interface size` (shortcuts in tooltip) | `Размер интерфейса` |
| `Editor layout` + `Classic table` / `Split stacked` / `Split side by side` | `Editor layout` + `Table` / `Stacked` / `Side by side` | `Расположение редактора` + `Таблица` / `Друг под другом` / `Рядом` |
| `Keyboard shortcuts` / `Customize shortcuts…` | `Keyboard shortcuts` / `Customize…` | `Сочетания клавиш` / `Настроить…` |
| `Theme could not be saved: {error}` / `Layout could not be saved: {error}` | `Couldn't save this setting.` + Details | `Не удалось сохранить настройку.` + Подробности |

### 9.2 AI

| Current | EN | RU |
|---|---|---|
| `AI inference` / `Where word timestamps, draft text and segment transcription run.` | `AI` / `Used for aligning words, drafting text and filling segments.` | `ИИ` / `Используется для выравнивания слов, черновиков и заполнения сегментов.` |
| `Where to draft` + `This PC` / `Remote GPU` | `Runs on` + `This computer` / `Remote server` | `Где выполняется` + `Этот компьютер` / `Удалённый сервер` |
| `Models on this PC` / `Gold's local models (downloaded once, about 2.6 GB) run on this PC's GPU through WebGPU.` | `Models on this computer` / `About 2.6 GB, downloaded once. Needs a graphics card that supports WebGPU.` | `Модели на этом компьютере` / `Около 2,6 ГБ, загружаются один раз. Нужна видеокарта с поддержкой WebGPU.` |
| statuses `installed and tested` / `installed, not tested yet` / `downloading` / `not installed ({x} GB to download)` / `download stopped` / `an older release is installed` / `model error` / `Checking this PC…` | `Ready` / `Installed — run the test` / `Downloading…` / `Not installed ({x} GB)` / `Download paused` / `Update available` / `Problem` / `Checking…` | `Готово` / `Установлено — запустите проверку` / `Загрузка…` / `Не установлено ({x} ГБ)` / `Загрузка остановлена` / `Доступно обновление` / `Ошибка` / `Проверка…` |
| `GPU: {vendor} {arch}` / `GPU: WebGPU ready` / `WebGPU with shader-f16 is unavailable` | `Graphics: {vendor} {arch}` / `Graphics: ready` / `This graphics card can't run the models` | `Видеокарта: {vendor} {arch}` / `Видеокарта: готова` / `Эта видеокарта не может запускать модели` |
| `Release {r}: {state}. {gpu}.` | `{state} · {gpu}` | |
| buttons `Install models` / `Reinstall / update` / `Test with sample` / `Remove models` / `Refresh` / `Cancel` | `Download` / `Update` / `Run test` / `Delete models` / `Check again` / `Cancel` | `Скачать` / `Обновить` / `Запустить проверку` / `Удалить модели` / `Проверить снова` / `Отмена` |
| progress `Downloading Gold runtime` / `Installing Gold runtime` / `Downloading models` / `finishing` / `{a} of {b}` | `Downloading runtime…` / `Installing runtime…` / `Downloading models…` / `Finishing…` / `{a} of {b}` | `Загрузка среды…` / `Установка среды…` / `Загрузка моделей…` / `Завершение…` / `{a} из {b}` |
| `Models installed. Run the test to finish setup.` / `Test passed: {n} words in {s} s of sample audio.` / `Models removed.` / `Model download stopped.` / `Cancelled.` | `Downloaded. Run the test to finish.` / `Test passed — {n} words from {s} s of audio.` / `Models deleted` / `Download stopped` / `Cancelled` | `Загружено. Запустите проверку.` / `Проверка пройдена — {n} слов из {s} с аудио.` / `Модели удалены` / `Загрузка остановлена` / `Отменено` |
| `Remote server (used when drafting on the remote GPU)` + `Remote server URL` + `Remote access key (optional)` + `Save URL` + `Enter an http(s) server address.` | *(Advanced)* `Remote server` + `Address` + `Access key (optional)` + `Save` + `Enter an address starting with http:// or https://.` | `Удалённый сервер` + `Адрес` + `Ключ доступа (необязательно)` + `Сохранить` + `Введите адрес, начинающийся с http:// или https://.` |
| `Remote GPU: checking…` / `Remote GPU ({release}): timestamps {avail}, draft text {avail}, denoise {avail}, error-task alignment {avail}` / `Remote GPU: release {r} is not ready.` / `Remote GPU: {message}` | `Remote server: checking…` / `Remote server: ready` (unavailable features listed in Details) / `Remote server: not ready` / `Remote server: {message}` | `Удалённый сервер: проверка…` / `… готов` / `… не готов` |
| `Gold Draft polish` / `Optional OpenRouter refinement after timestamps; the key also powers error-task Initial fill.` | `Polish` / `After drafting, an online model tidies the text. Needs an OpenRouter key.` | `Доработка текста` / `После черновика онлайн-модель приводит текст в порядок. Нужен ключ OpenRouter.` |
| `Polish draft text with OpenRouter` | `Polish drafts` | `Дорабатывать черновики` |
| `OpenRouter API key (sk-or-…)` / `OpenRouter keys start with sk-or- and contain no spaces.` / `Check backend` / `Backend reachable` / `Backend check failed` | `OpenRouter key` / `Keys start with sk-or-.` / `Test key` / `Key works` / `Key doesn't work` | `Ключ OpenRouter` / `Ключ начинается с sk-or-.` / `Проверить ключ` / `Ключ работает` / `Ключ не работает` |
| `Polish model` / `Service tier` + raw `default flex priority` / `Reasoning effort` + raw `none … xhigh` | *(Advanced)* `Model` / `Priority` + `Standard` `Cheaper` `Faster` / `Thinking` + `Default` `Off` `Minimal` `Low` `Medium` `High` `Very high` | `Модель` / `Приоритет` + `Обычный` `Дешевле` `Быстрее` / `Размышление` + `По умолчанию` `Выкл.` `Минимум` `Низкое` `Среднее` `Высокое` `Очень высокое` |
| `Denoise` / `Speech enhancement for the Original/Enhanced mix.` / `Where to denoise` / `Denoise automatically when a task opens` | `Denoise` / `Removes background noise. You can mix the original and cleaned audio in the editor.` / `Runs on` / `Denoise automatically when a project opens` | `Шум` / `Убирает фоновый шум. В редакторе можно смешивать исходный и очищенный звук.` / `Где выполняется` / `Убирать шум автоматически при открытии проекта` |
| `Gold is disabled. Re-enable Gold to manage inference settings.` | `The AI extension is turned off. Turn it on in Extensions to change these settings.` | `Дополнение ИИ выключено. Включите его в Расширениях.` |
| `AI settings could not be saved: {error}` | `Couldn't save AI settings.` + Details | `Не удалось сохранить настройки ИИ.` + Подробности |

### 9.3 Updates

| Current | EN | RU |
|---|---|---|
| `Updates` / `Sonus checks for updates on launch and every few hours.` | `Updates` / `Exaudio checks for updates automatically.` | `Обновления` / `Exaudio проверяет обновления автоматически.` |
| `Version` / `Build` / `Channel` / `Source` | `Version` / *(Build → tooltip)* / `Channel` / *(Source → tooltip)* | `Версия` / `Канал` |
| channel labels (from services) `stable` / `alpha` | `Stable` / `Early access` | `Стабильный` / `Ранний доступ` |
| `Check for updates` / `Install update` | `Check now` / `Update` | `Проверить` / `Обновить` |
| `Update channel could not be saved: {error}` | `Couldn't save this setting.` + Details | |

### 9.4 Babel (only when the Babel extension is on)

| Current | EN | RU |
|---|---|---|
| panel titles `Babel account and relay` / `Babel AI review` | `Babel` (one section) | `Babel` |
| `Sub-account relay` / `Run the bundled relay on this machine and remember its URL for sub-accounts.` | `Sub-account relay` / `Lets sub-accounts sign in through this computer.` | `Ретранслятор для под-аккаунтов` / `Позволяет под-аккаунтам входить через этот компьютер.` |
| `Relay URL, e.g. http://127.0.0.1:8787` / `Save URL` / `Start relay` / `Stop relay` / `Stopped` / `Starting…` / `Running at {url}` / `Failed: {e}` | `Address` / `Save` / `Start` / `Stop` / `Stopped` / `Starting…` / `Running at {url}` / `Not running` + Details | `Адрес` / `Сохранить` / `Запустить` / `Остановить` / `Остановлен` / `Запуск…` / `Работает: {url}` / `Не работает` |
| `Session & portability` / `Keep your Babel sign-in with this installation when you move it to another device.` / `Make installation portable` | `Sign-in` / `Keep your sign-ins with the app folder so they move with the app.` / `Store sign-ins next to the app` | `Вход` / `Хранить входы рядом с приложением, чтобы они переносились вместе с ним.` / `Хранить рядом с приложением` |
| `Babel task files` / `Open Babel JSON…` | **[gone]** (Start › Babel file tile) | |
| `Review Helper` / `Magic Review and the Grader use this OpenRouter key and model.` / `Magic Review mode` + `Interactive` / `Fast` / `Review OpenRouter API key (sk-or-…)` / `Review model (backend default)` / `Suggested: …` / `Model ids look like provider/model.` | `AI review` / `Used by Magic Review and the Grader.` / `Magic Review` + `Step by step` / `All at once` / `OpenRouter key` / `Model (optional)` / `Suggested: …` | `ИИ-ревью` / `Используется Magic Review и Grader.` / `Magic Review` + `Пошагово` / `Сразу всё` / `Ключ OpenRouter` / `Модель (необязательно)` |

---

## 10. AI (Gold) in the editor

### 10.1 Draft dialog

| Current | EN | RU |
|---|---|---|
| title `Gold Draft` / `Review before you save or submit.` | `AI draft` / `Check the result before you keep it.` | `Черновик ИИ` / `Проверьте результат, прежде чем оставить его.` |
| `Close` / tooltip `Close and cancel the run` | `Close` / `Close and cancel` | `Закрыть` / `Закрыть и отменить` |
| `Restore original` / `Restore the segments as the draft wrote them, before refinement` | `Undo polish` / `Go back to the text before polishing` | `Отменить доработку` / `Вернуть текст до доработки` |
| `Retry refinement` / `Apply draft` / `Done` | `Polish again` / `Keep draft` / `Done` | `Доработать снова` / `Оставить черновик` / `Готово` |
| phases `Reading transcript` / `Creating timestamps on this PC` / `… on the remote GPU` / `Drafting` / `Writing Draft text` / `Writing {n} segments` / `Refining` / `Capturing audio` / `{i}/{n} segments` / `connection lost, reconnecting` | `Reading transcript…` / `Aligning words on this computer…` / `… on the remote server…` / `Drafting…` / `Writing text…` / `Writing {n} segments…` / `Polishing…` / `Preparing audio…` / `{i} of {n} segments` / `Connection lost, reconnecting…` | `Чтение расшифровки…` / `Выравнивание слов на этом компьютере…` / `… на удалённом сервере…` / `Черновик…` / `Запись текста…` / `Запись {n} сегментов…` / `Доработка…` / `Подготовка аудио…` / `{i} из {n} сегментов` / `Связь потеряна, переподключение…` |
| `Timestamps ready for {n} track(s). Smart split, Alt+Click and auto-segmentation use them now.` | `Words aligned on {n} tracks. Smart split and Alt+click now snap to words.` | `Слова выровнены на {n} дорожках. Умное разделение и Alt+клик теперь привязаны к словам.` |
| `Draft kept. Refinement failed: {e} Apply the draft or retry refinement.` | `Draft is ready, but polishing failed. Keep the draft or try polishing again.` + Details | `Черновик готов, но доработка не удалась. Оставьте черновик или попробуйте снова.` |
| `Draft ready. {a} changed, {b} left as is.` / `Draft ready: {n} segments.` | `Draft ready — {a} segments changed, {b} unchanged.` / `Draft ready — {n} segments.` | `Черновик готов — изменено {a}, без изменений {b}.` / `Черновик готов — {n} сегментов.` |
| error titles `Set up the models on this PC first` / `This PC can't run the local models` / `No remote GPU is free right now` / `The server runs another model release` / `This backend can't draft` / `Cancelled` / `Gold Draft failed` | `Download the models first` / `This computer can't run the models` / `The remote server is busy` / `The server needs an update` / `Drafting isn't available here` / `Cancelled` / `Drafting failed` | `Сначала скачайте модели` / `Этот компьютер не может запускать модели` / `Удалённый сервер занят` / `Серверу нужно обновление` / `Черновик здесь недоступен` / `Отменено` / `Не удалось создать черновик` |
| `Gold Draft is unavailable: this task is read-only.` / `Waiting for audio: speaker audio is still loading. Try again in a moment.` / `Gold Draft needs the speaker audio, and none of it is loaded.` | `This project is read only.` / `Audio is still loading. Try again in a moment.` / `No audio is loaded for the speakers.` | `Этот проект только для чтения.` / `Аудио ещё загружается. Попробуйте через мгновение.` / `Аудио спикеров не загружено.` |
| `"Where to draft" changed during the run. Generate again.` / `The transcript changed while drafting. Generate again.` | `Settings changed during the run. Start again.` / `The transcript changed during the run. Start again.` | `Настройки изменились во время работы. Начните заново.` / `Расшифровка изменилась во время работы. Начните заново.` |
| `No polish provider is available for this document.` / `Refinement is off in settings.` / `Refinement needs an OpenRouter API key in settings.` | `Polishing isn't available for this project.` / `Polishing is turned off in Settings › AI.` / `Add an OpenRouter key in Settings › AI to polish.` | `Доработка недоступна для этого проекта.` / `Доработка выключена в Настройки › ИИ.` / `Добавьте ключ OpenRouter в Настройки › ИИ.` |
| `Apply refused: segments were added, removed, moved or resized since the draft was written.` | `The segments changed since the draft was made. Make a new draft.` | `Сегменты изменились после создания черновика. Сделайте новый черновик.` |
| `Applied to {n} segments.` / `Restored.` / `Gold Draft wrote {n} segments.` / `Gold Draft cancelled.` | `Draft kept for {n} segments` / `Polish undone` / `Draft written to {n} segments` / `Draft cancelled` | `Черновик оставлен для {n} сегментов` / `Доработка отменена` / `Черновик записан в {n} сегментов` / `Черновик отменён` |
| problem box `Set up models` / `Try again` / `Couldn't capture speaker audio. The draft may miss laughter and other sounds.` / `Captured audio for {a} of {b} speakers. …` / `Retry audio` / `Use captured audio` / `Continue without audio` | `Download models` / `Try again` / `Couldn't read the audio. The draft may miss laughter and other sounds.` / `Only {a} of {b} speakers have audio. …` / `Try again` / `Continue with what loaded` / `Continue without audio` | `Скачать модели` / `Попробовать снова` / `Не удалось прочитать аудио. В черновике могут отсутствовать смех и другие звуки.` / `Аудио есть только у {a} из {b} спикеров. …` / `Попробовать снова` / `Продолжить с тем, что загрузилось` / `Продолжить без аудио` |
| `Waiting for the first segment…` / `No changes.` | `Waiting for the first segment…` / `No changes` | `Ожидание первого сегмента…` / `Без изменений` |
| `Task {id}` pill | **[gone]** | |

### 10.2 Fill this segment (toasts)

| Current | EN | RU |
|---|---|---|
| `Transcribing on {place}…` | `Filling the segment on {place}…` | `Заполнение сегмента: {place}…` |
| `Transcribe segment: this track's audio is not loaded.` / `the backend returned no text for this segment.` / `Transcribe segment failed: {msg}` | `This track has no audio loaded.` / `Nothing was heard in this segment.` / `Couldn't fill the segment.` + Details | `У этой дорожки не загружено аудио.` / `В этом сегменте ничего не распознано.` / `Не удалось заполнить сегмент.` |

### 10.3 Denoise control

| Current | EN | RU |
|---|---|---|
| button `Denoise` / `Denoising` | `Denoise` / `Denoising…` | `Убрать шум` / `Удаление шума…` |
| tooltips `Generate enhanced (denoised) audio` / `Enhanced audio is ready` / `Denoise status` | `Remove background noise` / `Clean audio is ready` / `Status` | `Убрать фоновый шум` / `Очищенный звук готов` / `Состояние` |
| slider `Mix: Enhanced (left) ↔ Original (right)` / labels `Original` / `{p}% enhanced` / `{p}% enh.` | `Clean ↔ Original` / `Original` / `{p}% clean` | `Очищенный ↔ Исходный` / `Исходный` / `{p}% очищ.` |
| popover `No enhanced audio yet.` / `Runs on: {place}` / `{stage} · elapsed {t}` / `Using saved audio` / `Enhanced on {place} in {t}` / `Cancel` | `No clean audio yet` / `Runs on {place}` / `{stage} · {t}` / `Using saved result` / `Cleaned on {place} in {t}` / `Cancel` | `Очищенного звука пока нет` / `Выполняется: {place}` / `{stage} · {t}` / `Используется сохранённый результат` / `Очищено ({place}) за {t}` / `Отмена` |

### 10.4 Extension names and descriptions (Extensions list)

| Extension | EN name · description | RU |
|---|---|---|
| exaudio.local | **[gone]** from the list (it is the Start screen) | |
| exaudio.formats | `Subtitles and text` · `Import and export SRT, WebVTT and plain text.` | `Субтитры и текст` · `Импорт и экспорт SRT, WebVTT и обычного текста.` |
| exaudio.lint-basic | `Basic checks` · see §6 | |
| exaudio.gold | `AI tools` · `Align words, draft text, fill segments and remove noise — on this computer or a remote server.` | `Инструменты ИИ` · `Выравнивание слов, черновики, заполнение сегментов и удаление шума — на этом компьютере или удалённом сервере.` |
| exaudio.babel | `Babel` · `Take, edit and submit Babel tasks.` | `Babel` · `Брать, редактировать и сдавать задачи Babel.` |
| exaudio.babel-lint | `Babel checks` · `Babel's transcription rules with automatic fixes.` | `Проверки Babel` · `Правила транскрипции Babel с автоисправлениями.` |
| exaudio.babel-tags | `Babel tags` · `Tag completion, СКАЗ conversion and number tools for Babel tasks.` | `Теги Babel` · `Подсказки тегов, преобразование в СКАЗ и работа с числами для задач Babel.` |
| exaudio.babel-relay | `Babel relay` · `Sign in to Babel through a sub-account relay.` | `Ретранслятор Babel` · `Вход в Babel через ретранслятор для под-аккаунтов.` |
| example.hello-panel | `Hello panel` · `Example extension: inserts a greeting.` | `Панель-пример` · `Пример расширения: вставляет приветствие.` |

---

## 11. Babel workspace — headline strings

Babel is a specialist workflow; its vocabulary (claim, submit, stitched queue, QA, SKAZ,
annotation) is the platform's own and users know it. Rules 1, 3–6 still apply; the list
below is what a user sees before entering a task. The ≈ 430 task-panel strings follow
the glossary and are reviewed after this pass.

| Current | EN | RU |
|---|---|---|
| modes (Start tiles) | see §4.1 | |
| auth `Welcome back` / `Sign in with the phone number or e-mail of your Babel account.` / `Phone number or e-mail` / `Continue` / `Use sub-account code` | `Sign in to Babel` / `Use the phone number or e-mail of your Babel account.` / `Phone or e-mail` / `Continue` / `I have a sub-account code` | `Вход в Babel` / `Укажите телефон или e-mail аккаунта Babel.` / `Телефон или e-mail` / `Продолжить` / `У меня код под-аккаунта` |
| `Check your phone` / `Enter the verification code Babel sent you.` / `Verification code` / `Verify` / `Use a different number` | `Enter the code` / `Babel sent a code to your phone.` / `Code` / `Sign in` / `Use another number` | `Введите код` / `Babel отправил код на ваш телефон.` / `Код` / `Войти` / `Другой номер` |
| `Sign in to a sub-account` / `Use the relay address and one-time code from the account owner.` / `Relay URL, e.g. https://relay.example.com` / `One-time code` / `Sign in` / `Use Babel account` | *(same sign-in page, second method)* `Enter the relay address and the code from the account owner.` / `Relay address` / `Code` / `Sign in` / `Use a Babel account instead` | `Введите адрес ретранслятора и код от владельца аккаунта.` / `Адрес ретранслятора` / `Код` / `Войти` / `Войти через аккаунт Babel` |
| `Signing you in…` / `Your credentials stay on this device and are only used to talk to Babel.` / brand `Sonus` · `Babel workspace` | `Signing in…` / `Your sign-in stays on this device.` / `Exaudio` · `Babel` | `Вход…` / `Данные входа остаются на этом устройстве.` / `Exaudio` · `Babel` |
| dashboard `Projects` / `Choose a Babel project and claim the next available task.` / `Sub-accounts` / `Settings` / `Find task` / `Refresh` / `Start task` / `Ready to claim the next task` / `No claimable queue` / `{a} of {b} tasks today` / `Today` / `Available projects` / `Sign out` | `Projects` / `Pick a project and take the next task.` / `Sub-accounts` / `Settings` / `Find a task…` / `Refresh` / `Take next task` / `Tasks available` / `No tasks available` / `{a} of {b} today` / `Today` / `Projects` / `Sign out` | `Проекты` / `Выберите проект и возьмите следующую задачу.` / `Под-аккаунты` / `Настройки` / `Найти задачу…` / `Обновить` / `Взять задачу` / `Есть задачи` / `Нет задач` / `{a} из {b} сегодня` / `Сегодня` / `Проекты` / `Выйти` |
| problems `Your Babel session has expired` / `Babel no longer accepts the saved login. Sign in again to load your projects and claim tasks.` / `We couldn't refresh this workspace` / `The task could not be opened` / `{x} failed` | `Signed out of Babel` / `Sign in again to continue.` / `Couldn't refresh` / `Couldn't open the task` / `{x} didn't work` + Details | `Вы вышли из Babel` / `Войдите снова, чтобы продолжить.` / `Не удалось обновить` / `Не удалось открыть задачу` / `{x}: не удалось` |
| commands `Submit task` / `Skip chunk` / `Flag recording` / `Drop claim` / `Magic Review` / `Feedback` / `Grade review` / `Edit annotation` / `Autofix selected row` / `Autofix all rows` / `Convert selection to SKAZ` / `Replace selected spoken digits with {d}` / `Acknowledge warning` | `Submit` / `Skip` / `Flag recording` / `Release task` / `Magic Review` / `Feedback` / `Grade` / `Edit annotation` / `Fix this segment` / `Fix all segments` / `Convert to СКАЗ` / `Replace with {d}` / `Dismiss warning` | `Отправить` / `Пропустить` / `Пометить запись` / `Отпустить задачу` / `Magic Review` / `Отзыв` / `Оценить` / `Изменить аннотацию` / `Исправить сегмент` / `Исправить все сегменты` / `Преобразовать в СКАЗ` / `Заменить на {d}` / `Скрыть предупреждение` |
| file mode `Babel files` / `Open Babel JSON…` / `All workspaces` | **[gone]** — the tile opens the file picker directly | |
| `Save this Babel JSON file before leaving?` | `Save changes to “{file}”?` | `Сохранить изменения в «{file}»?` |

---

## 12. Error policy (applies to the ~600 unlisted error strings)

Every error the user sees is rendered by one component:

```
[icon]  {Headline — what happened, plain words, no colon-dumps}
        {Next step — one sentence, optional}
        ▸ Details          ← technical text, verbatim, copyable
```

| Source class | Headline | Details |
|---|---|---|
| Registry / activation bail (`Permissions must be approved before enabling …`, `Mode ID unavailable: …`, 30 more) | `This extension can't start.` / `Couldn't open this workspace.` | original message |
| WASM host (`explicit permission approval is missing`, `activation output queue unavailable`, …) | `{name} stopped working.` | original message |
| Format parser (`missing cue timing line`, 40 more) | `Couldn't read {file}.` + `Line {n}: {reason}` | — |
| Gold remote/local (`Staging audio failed: …`, `Remote drafting needs a volunteer …`) | existing sentences are already plain → keep as headline, strip prefixes | — |
| Save/IO | `Couldn't save “{project}”.` | OS error |
| Network | `Couldn't connect to {service}.` / `{service} isn't responding.` | HTTP status / error chain |

`{error:#}`, `{:?}`, raw IDs, SHA-256 and paths never appear in the headline.

---

## 13. Decisions (redline, 2026-11-06)

- `segment` / `сегмент`; `Extensions` / `Расширения` (not "Add-ons"); `под-аккаунт`; `спикер`; package extension `.exaudio`; tagline `From audio, every word.` / `Из звука — в слова.`
- Descriptions say what a thing does. No trailing "no X", "not Y", "nothing is Z" clauses anywhere in the UI.
