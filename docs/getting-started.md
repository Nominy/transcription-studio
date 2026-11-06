# Your first Exaudio project / Первый проект в Exaudio

[English](#english) · [Русский](#русский) · [Downloads / Загрузки](../README.md#download)

## English

### 1. Install

Use the stable [Windows x64 installer](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Setup.exe), or extract the [portable ZIP](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Portable.zip) to a writable folder and start `Exaudio.exe`.

Windows 10/11 x64 is the primary target. Packages are unsigned: SmartScreen may identify an unknown publisher. Check that the file came from `Nominy/exaudio` Releases and compare `SHA256SUMS` before deciding to run it. Do not disable Windows protection. No Rust installation is needed.

Alpha is experimental and uses a separate feed. Do not use an alpha build as your only copy of a project. See [release channels](releasing.md) for alpha and manual macOS packages.

### 2. Open audio from Start

Start lists one tile per installed mode. Choose **Local**, then **Open audio…**. Select one or more recordings; each file becomes an audio lane. WAV, MP3 and FLAC are good starting points. The local picker also supports OGG, M4A, MP4, WebM and MKV containers; successful decoding depends on the audio codec inside the file. A video container is an audio source here, not a video-editing workspace.

Use **Open project…** to continue a previously saved native project, **Recent** to reopen one you worked on before, or **Unsaved work › Restore** to pick up a recovery snapshot. Keep the project's original audio available. No account or AI configuration is needed for Local.

Platform modes such as **Babel** and **Babel error review** are optional tiles supplied by Exaudio add-ons. The first time you open one, Exaudio asks you to sign in (direct account or relay access from your operator); afterwards the tile opens straight into the mode. Accounts are managed under **Settings › Advanced › Accounts** (sign out, add another account). The integration is developed by the Exaudio author, not an official Babel product. A mode's required add-ons must remain enabled while that mode is open.

### 3. Listen and edit

Use playback and the waveform timeline to find speech. Create or adjust segments, choose speakers and edit transcript text. Work through the recording in short sections, listening again after timing or wording changes.

| Windows default | Action |
| --- | --- |
| `Space` | Play/pause outside text fields |
| `←` / `→` | Seek to previous/next segment outside text fields |
| `Ctrl+Z` | Undo in the focused editor context |
| `Ctrl+Shift+Z` or `Ctrl+Y` | Redo in the timeline context |
| `Ctrl+F` | Find in the transcript |
| `F3` / `Shift+F3` | Next/previous search match |

Keys are context-sensitive: a space should still type a space inside a text field. Open the keyboard shortcuts dialog for the full list and rebinding. Modified shortcuts replace these defaults.

**Optional AI:** enable/configure the AI extension and select a backend. For **This PC**, install the inference runtime and the models required by the chosen tools, then run the device/setup test. Windows local inference uses WebView2/WebGPU and needs compatible hardware and drivers. Models download separately; disk, RAM/VRAM and download requirements depend on the model. You can edit manually without them. Model output always needs human review.

Selecting a remote provider may upload audio or text to that provider. Inspect permissions and provider terms first; “Local mode” is not a promise that every optional extension is offline.

### 4. Save and export

Save the native project to keep editing later. Keep the source recordings with it and make ordinary backups. Subtitle exports are not complete project backups.

With **Formats** enabled, export **SRT**, **WebVTT** or **plain text** for delivery. Check the exported text and timing in the destination application. Plain text cannot preserve subtitle timing; subtitle formats cannot preserve every project detail or your source audio.

To import, open an audio project first and choose **Commands → Import transcript…**.
Timed cues are appended to the selected lane after confirmation; existing segments stay
unchanged. Plain text replaces the selected segment's text and keeps its timing.

### If something goes wrong

- **No sound:** check the selected output device, system volume, playback state and whether the source audio is still available.
- **No AI result:** confirm that the extension and backend are configured, installation has finished, and the device test succeeds. A model download is not part of the ordinary application installer.
- **A mode is unavailable:** check its required add-ons and service access. Local does not require signing in. If a platform tile keeps asking you to sign in, open **Settings › Advanced › Accounts** and sign out of the stale account first.
- **Need help:** open an [issue](https://github.com/Nominy/exaudio/issues) with the app/Windows version, mode, active extensions, reproduction steps and exact error. Attach only a synthetic, non-sensitive example; redact keys, account identifiers and private task content.

## Русский

### 1. Установка

Скачайте стабильный [установщик Windows x64](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Setup.exe) или распакуйте [portable ZIP](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Portable.zip) в папку с правом записи и запустите `Exaudio.exe`.

Основная платформа — Windows 10/11 x64. Пакеты не подписаны: SmartScreen может предупредить о неизвестном издателе. Перед запуском проверьте источник `Nominy/exaudio` Releases и опубликованный `SHA256SUMS`. Не отключайте защиту Windows. Устанавливать Rust не нужно.

Alpha — экспериментальный отдельный канал. Не храните единственную копию проекта только в альфа-сборке. Сведения об alpha и ручных пакетах macOS — в [описании каналов](releasing.md).

### 2. Открытие аудио с экрана «Начало»

На экране «Начало» — по плитке на каждый установленный режим. Выберите **Local → Открыть аудио…** и один или несколько файлов. Каждый файл станет аудиодорожкой. Начните с WAV, MP3 или FLAC. Также поддерживаются контейнеры OGG, M4A, MP4, WebM и MKV; возможность декодирования зависит от аудиокодека внутри файла. Видеоконтейнер здесь — источник звука, а не видеоредактор.

Кнопка **Открыть проект…** продолжает работу с сохранённым родным проектом, список **Недавние** открывает прежние проекты, а **Несохранённая работа › Восстановить** возвращает снимок восстановления. Сохраняйте доступ к исходному аудио. Для Local не нужны аккаунт и настройка ИИ.

Платформенные режимы **Babel** и **Babel error review** — необязательные плитки, которые добавляют дополнения Exaudio. При первом открытии Exaudio попросит войти (прямой аккаунт или доступ через relay от вашего оператора); дальше плитка открывает режим сразу. Аккаунты находятся в **Настройки › Дополнительно › Аккаунты** (выход, добавление ещё одного аккаунта). Интеграция — собственная разработка автора Exaudio, а не официальный продукт Babel. Пока режим открыт, требуемые ему дополнения должны оставаться включёнными.

### 3. Прослушивание и правки

Найдите речь с помощью воспроизведения и аудиоволн. Создавайте и уточняйте границы сегментов, выбирайте спикеров, правьте текст. Продвигайтесь небольшими фрагментами, повторно прослушивая запись после правок тайминга или формулировок.

| Windows, по умолчанию | Действие |
| --- | --- |
| `Space` | Воспроизведение/пауза вне текстовых полей |
| `←` / `→` | Предыдущий/следующий сегмент вне текстовых полей |
| `Ctrl+Z` | Отмена в активном контексте редактора |
| `Ctrl+Shift+Z` или `Ctrl+Y` | Повтор правки в контексте таймлайна |
| `Ctrl+F` | Поиск в транскрипции |
| `F3` / `Shift+F3` | Следующее/предыдущее совпадение |

Клавиши зависят от контекста: пробел в текстовом поле продолжает вводить пробел. Полный список и переназначение доступны в диалоге горячих клавиш. Пользовательские настройки заменяют эти значения по умолчанию.

**ИИ — по желанию:** включите и настройте ИИ-расширение, выберите бэкенд. Для **«Этот ПК»** установите среду выполнения и модели нужных инструментов, затем выполните тест устройства. Локальная обработка в Windows использует WebView2/WebGPU и требует совместимого оборудования и драйверов. Модели скачиваются отдельно; требования к диску, RAM/VRAM и объём загрузки зависят от модели. Можно редактировать вручную без ИИ. Результат модели всегда нужно проверять.

При выборе удалённого провайдера аудио или текст могут отправляться ему. Сначала изучите разрешения и условия: режим Local не гарантирует автономность любого дополнительного расширения.

### 4. Сохранение и экспорт

Сохраните родной проект для дальнейшего редактирования. Храните рядом исходные записи и делайте резервные копии. Экспорт субтитров не является полной копией проекта.

При включённом **Formats** экспортируйте **SRT**, **WebVTT** или **обычный текст**. Проверьте результат и тайминг в программе-получателе. Обычный текст не сохраняет тайминг субтитров; субтитры не сохраняют все детали проекта и исходное аудио.

Для импорта сначала откройте аудиопроект и выберите **Commands → Импорт текста…**.
Субтитры с таймингом добавляются на выбранную дорожку после подтверждения; существующие
сегменты не меняются. Обычный текст заменяет текст выбранного сегмента, сохраняя его тайминг.

### Если что-то не работает

- **Нет звука:** проверьте аудиовыход, системную громкость, состояние воспроизведения и доступность исходной записи.
- **Нет результата ИИ:** проверьте расширение, бэкенд, завершение установки и тест устройства. Модели не входят в обычный установщик приложения.
- **Недоступен режим:** проверьте необходимые дополнения и доступ к сервису. Local не требует входа. Если плитка платформы снова и снова просит войти, сначала выйдите из устаревшего аккаунта в **Настройки › Дополнительно › Аккаунты**.
- **Нужна помощь:** создайте [issue](https://github.com/Nominy/exaudio/issues) с версиями приложения/Windows, режимом, расширениями, шагами и точной ошибкой. Прикладывайте только синтетические примеры без чувствительных данных; удаляйте ключи, идентификаторы аккаунтов и приватное содержимое заданий.
