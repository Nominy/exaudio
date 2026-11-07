<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/header-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/header-light.svg">
  <img alt="Exaudio" src="assets/brand/header-light.svg" width="435">
</picture>

# Из звука — в слова.

Сегментный редактор транскрипций для Windows, macOS и Linux: слушайте, правьте тайминг, назначайте спикеров и редактируйте текст в одном нативном окне.
Локальные проекты не требуют аккаунта; ИИ, проверки и форматы — расширения, которые можно отключить.

[English](README.md) · **Русский** · [Сайт](https://nafty.is-a.dev/exaudio/) · [Начало работы](docs/getting-started.md#русский) · [Выпуски](https://github.com/Nominy/exaudio/releases) · [Issues](https://github.com/Nominy/exaudio/issues)

![Редактор Exaudio: синтетическое интервью после черновика ИИ — волна на таймлайне и 11 сегментов со спикером и временем](site/assets/editor-interview-draft.webp)

## Скачать

| Платформа | Стабильная версия (GitHub Releases) |
| --- | --- |
| Windows x64, установщик | [Exaudio-win-Setup.exe](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Setup.exe) |
| Windows x64, portable-архив | [Exaudio-win-Portable.zip](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Portable.zip) |
| macOS, Apple silicon | [Exaudio-mac-arm64.dmg](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-mac-arm64.dmg) |
| macOS, Intel | [Exaudio-mac-x64.dmg](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-mac-x64.dmg) |
| Linux x64 | [Exaudio-linux-x64.AppImage](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-linux-x64.AppImage) |
| Linux arm64 | [Exaudio-linux-arm64.AppImage](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-linux-arm64.AppImage) |
| Контрольные суммы | [SHA256SUMS](https://github.com/Nominy/exaudio/releases/latest/download/SHA256SUMS) |
| Установщик alpha для Windows (отдельная лента) | [Exaudio-alpha-Setup.exe](https://reviewgen.ovh/exaudio/alpha/win-x64/Exaudio-alpha-Setup.exe) |

Стабильные выпуски — теги `v*` в GitHub Releases; каждый выпуск содержит все платформы и файл `SHA256SUMS`, установленные копии обновляются сами. Alpha раздаётся с отдельного VPS (Windows и macOS), а не из стабильного канала обновлений; перед тестированием сделайте резервные копии проектов. См. [распространение и обновления](docs/releasing.md).

**Пакеты не подписаны.** Windows SmartScreen может предупредить о неизвестном издателе («Подробнее → Выполнить в любом случае»). macOS при первом запуске не сможет проверить приложение: откройте **Системные настройки → Конфиденциальность и безопасность** и нажмите **Всё равно открыть**. AppImage для Linux нужно сделать исполняемым (`chmod +x Exaudio-linux-x64.AppImage`; в большинстве дистрибутивов нужен FUSE 2, пакет `libfuse2`). Скачивайте только из Releases проекта, сверяйте запись в `SHA256SUMS` и не отключайте защиту ОС.

Требования: Windows 10/11 x64, macOS 14+ или 64-битный Linux с графическим окружением (X11 или Wayland, PulseAudio/ALSA), аудиовыход и место с правом записи для проектов. Локальный ИИ («Этот ПК») работает только в Windows: ему дополнительно нужны WebView2, видеокарта с поддержкой WebGPU и актуальными драйверами, а также отдельно скачанные модели (со своими лицензиями; размер, RAM/VRAM и диск зависят от модели); в macOS и Linux используйте удалённый сервер. Интернет нужен для загрузок, обновлений и удалённых сервисов, но не для правки локальных записей.

## Возможности

- **Сегменты и спикеры.** У каждой реплики — спикер, начало и конец. Разделяйте по курсору, объединяйте соседние, подрезайте по звуку, переименовывайте или добавляйте спикеров из меню строки.
- **Таймлайн и волна.** По дорожке на каждый аудиофайл, масштабируемая волна, обзорная полоса, скорость воспроизведения, повтор, отмена/повтор правок. Выделите участок волны — получится сегмент.
- **Локальный ИИ.** Выравнивание слов, черновик расшифровки, заполнение пропусков и шумоподавление — на вашей видеокарте через WebView2/WebGPU или на удалённом сервере. Любой результат — черновик для проверки.
- **Проверки.** Колонка «Проверки» отмечает пустой текст, наложения на одной дорожке и лишние пробелы и предлагает безопасные быстрые исправления. Расширения добавляют свои правила.
- **Импорт и экспорт.** SRT, WebVTT и обычный текст в обе стороны через расширение Formats (**Commands → Импорт текста…** / **Export…**). Папка родного проекта хранит копию аудио и все сегменты.
- **Расширения.** Команды, панели, проверки, форматы и режимы — это расширения: встроенные написаны на Rust, устанавливаемые — WASM в песочнице.
- **Палитра команд (Ctrl+Shift+P).** Все команды редактора и расширений в одном списке с поиском по названию и сочетанием клавиш рядом.

На экране «Начало» — по плитке на каждый установленный режим. **Local** открывает аудио и проекты на этом компьютере без аккаунта. **Babel**, **Babel error review** и **Babel file** — необязательные режимы из авторской интеграции Babel; им нужен собственный доступ к сервису, для Local они не требуются. Exaudio — не официальный продукт Babel и не заявляет о его поддержке.

## Быстрый старт

1. Запустите установщик или распакуйте portable-архив (Windows), перетащите `Exaudio.app` в «Программы» (macOS) или сделайте AppImage исполняемым и запустите его (Linux).
2. На экране «Начало» выберите **Local → Открыть аудио…**. Каждый файл станет дорожкой; начните с WAV, MP3 или FLAC (контейнеры OGG, M4A, MP4, WebM и MKV тоже открываются — в зависимости от кодека внутри).
3. Слушайте и правьте: выделите участок волны, чтобы добавить сегмент; `S` разделяет по курсору, `D` удаляет, `Space` — воспроизведение, `←`/`→` — переход по сегментам, `Ctrl+Z`/`Ctrl+Y` — отмена/повтор, `Ctrl+F` — поиск. Полный список с переназначением — в диалоге горячих клавиш.
4. ИИ по желанию: в ИИ-расширении выберите **Этот ПК** (установите среду выполнения и модели, выполните тест устройства) или удалённый сервер, затем используйте **Align words**, **Draft with AI** или **Denoise** и проверьте результат, прежде чем оставить его.
5. **Сохраните** родной проект (папка с `project.json` и `audio/`), а при необходимости **Export…** в SRT, WebVTT или TXT. Снимки несохранённой работы, настройки и расширения хранятся в `%APPDATA%\Exaudio` (`~/Library/Application Support/Exaudio` в macOS, `~/.config/Exaudio` в Linux).

Пошагово: [Начало работы](docs/getting-started.md#русский).

## Расширения

Устанавливаемые расширения — пакеты `.exaudio` (WebAssembly-компоненты), которые собираются с [Rust SDK](sdk/rust/README.md) по типизированному [WIT API](wit/exaudio-extension.wit). Они добавляют команды, панели, проверки, форматы и режимы, а доступ к документам, аудио, хранилищу и сетевым хостам получают только по явным разрешениям. Встроенные нативные расширения работают с правами приложения.

Открытый каталог — [`Nominy/exaudio-extensions`](https://github.com/Nominy/exaudio-extensions) ([`index.json`](https://raw.githubusercontent.com/Nominy/exaudio-extensions/main/index.json)). Авторы хранят исходники и версии пакетов в собственных репозиториях; индекс перечисляет выпуски, совместимость, хеши и запрашиваемые разрешения. Перед установкой проверяйте автора, код и разрешения — запись в каталоге не гарантирует безопасность.

## Сборка из исходников

На любой платформе нужны Git, Rust из `rust-toolchain.toml` и CMake (встроенный libopus собирается из исходников). Windows добавляет инструменты MSVC C++ с Windows SDK; macOS — инструменты командной строки Xcode; Linux — компилятор C и dev-пакеты из `.github/actions/linux-build-deps/action.yml` (xkbcommon, Wayland, X11/XCB, Vulkan, fontconfig, ALSA, D-Bus). Внутренние крейты сохраняют префикс `studio-*`.

```powershell
git clone https://github.com/Nominy/exaudio.git
cd exaudio
cargo run -p studio-app
```

Стандартная сборка включает Local, Formats, базовые проверки, ИИ-инструменты Gold и интеграцию Babel и собирается из чистого клона. Подмножества features:

```powershell
# ИИ-инструменты без Babel
cargo run -p studio-app --no-default-features --features local,formats,basic-checks,gold --locked
# Только локальный редактор
cargo run -p studio-app --no-default-features --features local,formats,basic-checks --locked
```

Приватный пакет данных Babel (`tools/babel-data.json`) нужен только инструментам сверки и тестам сверки линтера; без него они пропускаются. Проверки, процесс для участников и SDK расширений — в [руководстве разработчика](docs/development.md).

## Участие

Приветствуются сообщения об ошибках, код, расширения, переводы и улучшения доступности. В отчётах и тестах используйте синтетические записи; не прикладывайте ключи, приватные задания и чужое аудио. Крупные изменения сначала обсудите в [issue](https://github.com/Nominy/exaudio/issues).

## Лицензия

[MIT](LICENSE), включая собственные расширения автора. Зависимости, веса моделей и сторонние пакеты расширений сохраняют свои лицензии. Основа — Rust, GPUI / gpui-kit и Wasmtime; надпись использует контуры букв Outfit (SIL OFL 1.1). См. [благодарности](docs/development.md#licenses-and-acknowledgements).
