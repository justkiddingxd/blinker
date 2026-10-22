# Changelog

Each version's section becomes its release notes on GitHub (`.github/workflows/release.yml` takes the part under `## <version>`).

## 0.3.4

**Мак: Liquid Glass везде (macOS 26)**
- Окна настроек, истории и редакторов теперь из матового стекла: под ними размыто видно рабочий стол, сайдбар и заголовки — отдельные стеклянные панели.
- Выделение в меню, переключатели и кнопки стали капсулами, карточки настроек полупрозрачными.
- Стеклянные: подсказки знакомства, «что нового», уведомления в окне, диалоги, выпадающие списки, меню и плашка записи.

**Везде**
- Окна настроек и истории идут на 120 Гц: на Metal (мак) и DirectX (Windows) вместо OpenGL, который на маке упирался в 60 и дёргался.
- Клавиши действий в захвате (копировать, сохранить, править, закрепить, красивый снимок, загрузить) можно поменять в «горячие клавиши → в захвате». Подсказки на кнопках показывают твои клавиши.
- Плашка в переключателях (язык, кодек, качество…) плавно переезжает к выбранному варианту.
- Знакомство показывается один раз, даже если его включили снова руками или закрыли захват через Esc. Работает и в мгновенном режиме.
- Меньше места между разделами в меню настроек.
- Новая версия предлагается тихой строкой «обновить до …» внизу меню вместо яркой кнопки; проверка обновлений — в «система».
- Выбор звука снимка — в одну строку, громкость и проценты выровнены, ползунки новые: тонкая дорожка и круглая ручка.

---

**Mac: Liquid Glass everywhere (macOS 26)**
- The settings, history and editor windows are frosted glass: the desktop shows through, blurred, and the sidebar and title bars are panes of glass floating in them.
- The menu selection, switches and buttons are capsules, the settings cards are see-through.
- Glass: the walkthrough's bubbles, "what's new", in-window notes, dialogs, dropdowns, menus and the recording pill.

**Everywhere**
- The settings and history windows run at 120 Hz: drawn with Metal (Mac) and DirectX (Windows) instead of OpenGL, which stopped at 60 and juddered on a Mac.
- The keys of a capture's actions (copy, save, edit, pin, pretty shot, upload) can be changed in "hotkeys → in a capture". The buttons' tips show your keys.
- The plate of a switch (language, codec, quality…) glides to the chosen option.
- The walkthrough shows once, even when turned on again by hand or when the capture is closed with Esc. It works in instant mode too.
- Less space between the sections of the settings menu.
- A new version is offered by a quiet "update to …" row at the bottom of the menu instead of a bright button; checking by hand is on the "system" page.
- The capture sound is picked in one row, the volume and its percent line up, and the sliders are new: a thin track and a round knob.

## 0.3.3

- «Выход» в трее закрывает всё: открытые настройки, историю, редакторы и закреплённые снимки. Раньше они оставались висеть после выхода. Закрыть blinker можно и командой `blinker --quit`.
- В истории файлы можно переименовать двойным кликом по имени. Enter сохраняет, Esc отменяет.
- Уведомление отвечает на нажатия: кнопка вдавливается, «копировать» сменяется галочкой, над картинкой появляется плашка с тем, что случилось. Над картинкой рука вместо стрелок «переместить».
- Сохранённый повтор встречает надпись «Clipped!» над уведомлением.
- «Уведомление» переехало в раздел «общее»: оно и для снимков, и для записей.
- Первый снимок проводит по шагам: как выделить, чем рисовать, что сделать со снимком. Повторить можно в «захват → знакомство».
- Уведомление показывает снимок целиком, а не обрезанный кусок.
- Панели при выделении меньше рядом с маленькой областью и больше не наезжают друг на друга.
- Длительность повтора вводится в секундах, рядом видно, сколько это («≈ 5 мин»).
- Статус в Discord называет игру: «Clipping Dota 2».
- В настройках страницы прокручиваются под кнопками окна, а не поверх них.
- post.blinker.cool: файлы больше 25 мб живут сутки, а когда место кончается, первыми удаляются самые старые, и загрузка не отказывает.

---

- "Quit" in the tray closes everything: open settings, history, editors and pinned shots. They used to stay behind. `blinker --quit` does the same.
- Files can be renamed in the history with a double click on the name. Enter keeps the name, Esc drops it.
- The card answers clicks: the button presses in, "copy" turns into a tick, a chip over the picture says what happened. The picture shows a hand, not the "move" arrows.
- A saved replay gets "Clipped!" over its card.
- "Notification" moved to "general": it is for screenshots and recordings alike.
- The first capture walks you through: selecting, drawing, what to do with the shot. Again any time from "capture → walkthrough".
- The card shows the whole capture, not a cropped part.
- The capture panels shrink next to a small selection and no longer run into each other.
- The replay length is typed in seconds, with what that makes beside it ("≈ 5 min").
- The Discord status names the game: "Clipping Dota 2".
- Settings pages scroll under the window buttons, not over them.
- post.blinker.cool: files over 25 mb live one day, and a full disk makes room by dropping the oldest files instead of refusing uploads.

## 0.3.2

**Мак**
- Панели инструментов при выделении теперь из Liquid Glass (macOS 26): под ними видно размытый экран.

**Сайт и README**
- Анимации пересняты с новыми панелями, добавлена третья: мгновенный повтор.
- На сайте превью сразу под шапкой, справа от текста; футажи листаются стрелками (и клавишами ← →), под ними подпись.
- blinker.cool переделан в стиле приложения: сайт сам узнаёт систему и предлагает нужную версию, установка и горячие клавиши показываются для неё (с переключателем macOS / Windows).

## 0.3.1

**Обновления**
- Кнопка «проверить обновление» в разделе «система» и внизу боковой панели.
- Настройки проверяют обновление каждый раз, когда открываются (не чаще раза в 10 минут), а не раз в 6 часов.
- Новая версия узнаётся с blinker.cool, GitHub — запасной вариант.

**Запись и звук**
- Выбор микрофона в настройках (для записи и повтора).
- На Windows редактор видео путал дорожки «звук системы» и «микрофон»: теперь они различаются по самому файлу.
- Новый вид значка записи: карточка с красной плашкой времени и красной кнопкой «стоп».

**Редактор видео**
- Кнопка «загрузить» рядом с «сохранить mp4»: правленый клип сначала сохраняется, потом загружается вместе с метками.
- Края обрезки берутся надёжно: широкая зона захвата, время края над ним, пока тянешь.
- Рамка оставляемого отрезка проходит через все дорожки, подсказки понятнее.

**ffmpeg**
- Раздел «ffmpeg» в «системе»: blinker сам находит ffmpeg (Homebrew, winget, scoop, PATH…), можно указать папку.
- Нет ffmpeg — скачивается одной кнопкой прямо там, где он пригодится (сжатие для telegram).

**История**
- Новое окно «история» (пункт в меню трея): последние снимки и записи по дням с превью — открыть, скопировать, загрузить, показать в папке.
- Вкладка «загрузки»: всё загруженное со ссылками и сроком жизни, ссылку можно скопировать или открыть, а файл — удалить с сервера. Открывается и из настроек загрузки.

**Мак**
- Liquid Glass на macOS 26: стеклянная боковая панель настроек, стеклянные полосы сверху у редакторов и истории, стеклянное уведомление после снимка, иконка подстраивается под стиль системы (тёмная, тонированная, прозрачная).
- Пока открыто меню в строке меню, горячие клавиши всё равно снимают: меню закрывается.
- Двойной клик по значку убран: на маке первый клик и так открывает меню.
- В лупе текст больше не наезжает друг на друга: длинные координаты (Retina) уходят на вторую строку.

**Прочее**
- Нажми на версию внизу боковой панели: откроется история версий.
- Ctrl+C (на маке ⌘C) копирует выделенную область, если мгновенный режим выключен. Раньше это сочетание не срабатывало, то же в редакторе и у закреплённых снимков.
- Статус в Discord: «Clipping with blinker» с логотипом и кнопкой на blinker.cool (галка в «системе»).
- Windows: у blinker.exe своя иконка, а blinker находится в поиске Windows (ярлык в меню «Пуск»).
- post.blinker.cool: картинки в превью Telegram и других мессенджеров видны целиком.

---

**Updates**
- A "check for an update" button in "system" and at the bottom of the sidebar.
- The settings check for an update every time they open (at most every 10 minutes), not every 6 hours.
- The new version is learned from blinker.cool, with GitHub as a fallback.

**Recording and sound**
- Pick the microphone in the settings (for recordings and the replay).
- On Windows the video editor mixed up "desktop sound" and "microphone": now the file itself tells them apart.
- A new look for the recording indicator: a card with a red time chip and a red stop button.

**Video editor**
- "Upload" next to "save mp4": an edited clip is saved first, then uploaded with its markers.
- The trim edges are easy to grab: a wide grab zone, the edge's time above it while you drag.
- The frame of the kept part runs through every track, the hints are clearer.

**ffmpeg**
- An "ffmpeg" section in "system": blinker finds ffmpeg itself (Homebrew, winget, scoop, PATH…), or takes a folder.
- No ffmpeg: one button gets it, right where it helps (compressing for telegram).

**History**
- A new "history" window (in the tray menu): the latest screenshots and recordings by day with previews — open, copy, upload, show in the folder.
- An "uploads" tab: everything uploaded with its link and how long it lives; copy or open the link, delete the file from the server. It opens from the upload settings too.

**Mac**
- Liquid Glass on macOS 26: a glass settings sidebar, glass title bars in the editors and the history, and a glass card after a capture, and the icon follows the system's style (dark, tinted, clear).
- With the menu bar menu open the hotkeys still capture: the menu closes.
- No double click on the icon: on a Mac the first click opens the menu anyway.
- The loupe's caption no longer overlaps: long (Retina) coordinates go to a second line.

**Other**
- Click the version at the bottom of the sidebar: the release notes open.
- Ctrl+C (⌘C on a Mac) copies the selected area when instant mode is off. The shortcut didn't work before, in the editor and on pinned shots either.
- Discord status: "Clipping with blinker" with the logo and a blinker.cool button (a switch in "system").
- Windows: blinker.exe has its icon, and Windows search finds blinker (a Start menu shortcut).
- post.blinker.cool: pictures show whole in Telegram's and other messengers' previews.

## 0.3.0

**Загрузка**
- Сервер по умолчанию — **post.blinker.cool**: без регистрации, файлы живут 3 дня, видео до 100 мб. У каждой ссылки своя страница: просмотр картинки, плеер видео и звука.
- Свой такой же сервер поднимается одной командой: `docker compose up` в папке `server` ([инструкция](https://github.com/justkiddingxd/blinker/blob/main/server/README.ru.md)).
- Загрузчики ShareX: подключи файл `.sxcu` (раздел «система»), и снимки уходят на твой Zipline, XBackBone и другие.
- Загрузку можно выключить совсем: пропадут все её кнопки и клавиша `u`.

**Переезд с ShareX**
- Перенос настроек ShareX одной кнопкой: горячие клавиши, папка, формат, что делать после снимка, уведомление, звук, загрузчик. При первом запуске рядом с ShareX настройки сами открываются на этой кнопке.

**Видео**
- Метки на таймлайне редактора видео (клавиша `m`) с текстом. После загрузки они видны на post.blinker.cool: на полосе плеера и списком под видео.
- Видеодорожку можно выключить и сохранить только звук: mp3, m4a или wav.
- Мгновенный повтор: длительность вводится в минутах (до 30), а ещё можно включить «сохранение начинает повтор заново».
- Пока идёт повтор, иконка в трее красная.

**Вид**
- Новые настройки: боковая панель с разделами по группам, у повтора свой раздел, новые переключатели и карточки.
- Новое уведомление после снимка.
- Меню в трее с крупными плитками: область, экран, окно, видео, gif.
- Панель инструментов при скриншоте крупнее и уже: цвета свёрнуты в один кружок.
- На маке родные кнопки окна; окна больше не растягиваются и не разворачиваются на весь экран; текст в интерфейсе не выделяется.
- В мгновенном режиме панель инструментов стоит на месте и не уезжает за областью.

**Система**
- Автообновление: галка в разделе «система» (по умолчанию выключена). Новая версия ставится в фоне, blinker перезапускается, но не во время записи.
- После обновления настройки один раз показывают «что нового».
- Двойной клик по значку в трее — своё действие.

---

**Uploads**
- The default server is **post.blinker.cool**: no sign-up, files live 3 days, videos up to 100 mb. Every link opens a page of its own: an image viewer, a video and sound player.
- The same server runs anywhere with `docker compose up` in the `server` folder ([how](https://github.com/justkiddingxd/blinker/blob/main/server/README.md)).
- ShareX uploaders: pick an `.sxcu` file ("system") and captures go to your Zipline, XBackBone and the like.
- Uploads can be switched off altogether: every upload button and the `u` key go away.

**Moving from ShareX**
- ShareX settings in one click: hotkeys, folder, format, after-capture steps, notification, sound, uploader. On the first start next to ShareX the settings open at that button.

**Video**
- Markers with notes on the video editor's timeline (`m`). Uploaded, they show on post.blinker.cool: on the player's bar and as a list under the video.
- Switch the video track off to save the sound alone: mp3, m4a or wav.
- Instant replay: the length is typed in minutes (up to 30), and saving can start the replay over.
- The tray icon is red while the replay records.

**Look**
- New settings: a sidebar with grouped sections, the replay in a section of its own, new switches and cards.
- A new card after a capture.
- The tray menu has big tiles: region, screen, window, video, gif.
- The capture toolbar is bigger and narrower: the colors fold into one swatch.
- Native window buttons on the Mac; windows keep their size and don't go full screen; interface text can't be selected.
- In instant mode the capture toolbar stays put instead of following the selection.

**System**
- Auto update: a switch in "system" (off by default). A new version is put in place in the background and blinker restarts, never during a recording.
- After an update the settings show "what's new" once.
- A double click on the tray icon has its own action.

## 0.2.0

- blinker for macOS: screenshots and screen recording, the menu bar icon, global hotkeys, the capture overlay on every display.
- Updates in one click from the settings: download, replace, restart.
- Releases are built by GitHub Actions: `blinker.exe`, `Blinker.dmg`, `Blinker-macos.zip`.
- The outline rectangle in the capture toolbar.
- The site [blinker.cool](https://blinker.cool).

## 0.1.0

- The first version: screenshots and screen recording for Windows.
