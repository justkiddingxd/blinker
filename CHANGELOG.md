# Changelog

Each version's section has a Russian half, `---`, then an English one: the app shows the one of its language, and GitHub's release notes (and the Discord note) are the English half (`.github/workflows/release.yml`).

## 0.4.1

**Редактор видео**
- Редактор стал полноценным: полоса инструментов (разрезать, убрать кусок, текст, субтитр, из речи, метка, импорт, отменить и вернуть), справа инспектор с вкладками «видео», «текст», «звук», внизу таймлайн с линейкой времени, дорожками «текст» и «субтитры» и приближением (Ctrl+колесо, кнопки + и −, «весь клип»; обычное колесо листает).
- Текст поверх видео: заголовки, подписи, реакции на своей дорожке. Перетаскивается прямо на превью, у каждого свой вид и появление: проявление, всплытие, снизу, печать. Те же анимации есть у субтитров.
- Приближение кадра (масштаб и место) и «плавный наезд»: камера медленно приближается за весь клип, как в коротких видео.
- Монтаж: S — разрез на месте курсора, Del — убрать кусок между разрезами (или вернуть его), Ctrl+Z / Ctrl+Shift+Z — отменить и вернуть. Убранные куски пропускаются при просмотре и не попадают в mp4, gif и звук.
- Вертикальное видео 9:16 для Shorts, Reels и TikTok, 1080 × 1920. «Часть» — вертикальная полоса из кадра, рамку двигаешь на превью; «целиком» — весь кадр по центру на размытом фоне. Больше 60 кадров в секунду такие видео не хранят.
- Скорость куска: 0,5×, 1,5×, 2×, 3× в инспекторе на вкладке «видео». Звук ускоряется и замедляется вместе с картинкой, а голос в сохранённом клипе не пищит и не басит. На кадрах видна плашка со скоростью.
- Импорт: музыка (mp3, wav, m4a, flac…) на своей дорожке под клипом — её блок двигается, у неё громкость и затухание в конце; картинки (png, jpg, webp) поверх видео на дорожке «картинки» — двигаются на превью, у каждой размер и появление. Всё это попадает в mp4.
- Субтитры: дорожка под кадрами, T добавляет субтитр на месте курсора, края тянутся, блок двигается. Они впекаются в mp4.
- Вид субтитров: размер, шрифт (обычный, жирный, моно), ЗАГЛАВНЫЕ, фон (плашка, обводка, тень), цвет, место. Текст на превью перетаскивается вверх-вниз. Вид запоминается.
- Субтитры из речи: «субтитры» → «распознать речь». Распознаёт whisper прямо на компьютере, без интернета и без загрузки клипа куда-либо; язык определяется сам. Модель на выбор: «быстрее» (190 мб) или «точнее» (570 мб); ставится и удаляется в настройках → система или по кнопке в редакторе, сама по себе ничего не качается. Слушает включённые дорожки: выключи звук игры, чтобы остался один голос. Тихий микрофон подтягивается по громкости, тишина и выдуманные фразы («Продолжение следует…») отбрасываются.

**Снимки и клипы**
- Текст с экрана: кнопка «распознать текст» в панели после выделения (клавиша T) и своя клавиша, которая сразу выделяет и распознаёт (по умолчанию Ctrl+Alt+T, на маке Ctrl+Opt+Cmd+T). Окно встаёт на место области: каждая строка лежит поверх своих слов на тёмной плашке, текст выделяется и копируется, весь он сразу в буфере. На Windows читает PaddleOCR (PP-OCRv5): русский, украинский, белорусский и английский вперемешку, в том числе мелкий текст тёмных интерфейсов. Модель весит 13 мб и скачивается один раз: окно текста предложит её при первом распознавании, а ставится и удаляется она в настройках → система. Работает на компьютере, через встроенный в Windows движок, картинка никуда не уходит. На маке читает Vision. У окна шапка с числом строк и кнопкой «копировать всё», строки появляются по очереди, у каждой при наведении своя кнопка копирования.
- Видео по ссылке blinker.cool играет прямо в Discord и Telegram: у страницы есть размер видео и превью (первый кадр), а сервер переставляет индекс mp4 в начало файла, чтобы мессенджер мог играть его, не скачивая целиком.
- QR-коды на экране: при снимке найденный код обводится рамкой с бегущей полосой, под ним карточка — куда он ведёт, кнопка «перейти» и копирование ссылки (или «копировать», если там не ссылка). На маке карточка из стекла, как и окно распознанного текста. Выключается галкой «читать qr-коды» в настройках → захват.
- Пипетка: своя клавиша (по умолчанию Ctrl+Alt+C, на маке Ctrl+Opt+Cmd+C). Экран замирает, лупа у курсора, клик копирует цвет в hex.
- Клипы по играм: запись, повтор и gif во время игры называются по ней («Dota 2 2026-10-23 20-03-05.mp4») и лежат в «games / Dota 2». В истории на вкладке видео фильтр по играм. Выключается в «хранение». Пока на Windows.

**Логотип**
- Новый логотип: белая «B» с вырезанной звездой и красной точкой. На Windows классический, на маке стеклянный (Liquid Glass в macOS 26).
- Значок в трее и в строке меню мака — белая звезда на тёмной плитке, читается даже в 16 px. Пока пишется повтор, в углу красная точка (раньше значок краснел целиком).
- Статус в Discord с новым логотипом, без точки: она не спорит со значком записи в углу.

**Прочее**
- После автообновления blinker перезапускается тихо, в трее: окно настроек больше не открывается само и не выбивает из полноэкранной игры.
- «Настройки» и «история» из трея поднимают уже открытое окно наверх, даже свёрнутое.
- Меню трея собирается под себя: настройки → «меню трея». Меню показано как есть, пункты перетаскиваются между плитками сверху, списком и запасными справа; ряды плиток добавляются, их заголовки меняются по клику, есть разделители; «сбросить» возвращает меню по умолчанию. В меню теперь можно поставить «распознать текст» и пипетку. Клики по значку переехали туда же.
- Настройки разложены заново: «текст и qr-коды», «редактор видео» (открывать после записи, субтитры из речи), «меню трея», «интеграции» (Discord, ShareX, NVIDIA). Звук щелчка теперь в «захвате», в «системе» остались язык, запуск и обновления.
- Esc больше не закрывает историю. Правый клик по уведомлению закрывает его.
- Внизу настроек ссылки на GitHub и на сервер blinker в Discord (открывается сразу в приложении Discord, если оно стоит).
- Ссылки blinker://update: кнопка «Update» в Discord blinker открывает настройки и сразу ставит новую версию (через blinker.cool/update, браузер спросит, открыть ли blinker).
- Мак: клик выделяет часть окна, как на Windows. Нужен «универсальный доступ»: в настройках, раздел «захват», кнопка «разрешить».
- Мак: плашка записи всего экрана в режиме «в углу» стоит справа снизу, над Dock (раньше уезжала наверх).

---

**Video editor**
- A real editor now: a tool strip (cut, take out, text, subtitle, from speech, marker, import, undo and redo), an inspector on the right with "video", "text" and "sound" pages, and a timeline with a time ruler, "text" and "subtitles" lanes and zoom (Ctrl+wheel, the + and − buttons, "whole clip"; a plain wheel scrolls along).
- Texts over the video: titles, callouts, reactions on a lane of their own. Drag them on the preview; each has its look and its entrance: fade, pop, slide up, typing. Subtitles have the same animations.
- Zoom on the picture (scale and place) and a "slow push in": the camera moves in slowly over the whole clip, as short videos do.
- Cutting: S cuts at the playhead, Del takes the piece between cuts out (or brings it back), Ctrl+Z / Ctrl+Shift+Z undo and redo. Pieces taken out are skipped while playing and left out of the mp4, gif and sound.
- Vertical 9:16 video for Shorts, Reels and TikTok, 1080 × 1920. "Slice" takes an upright strip of the picture (drag the frame on the preview); "whole" puts all of it in the middle over a blurred copy. Such videos keep at most 60 frames a second.
- Piece speed: 0.5×, 1.5×, 2×, 3× on the inspector's "video" page. The sound speeds up and slows down with the picture, and voices in the saved clip keep their pitch. A badge on the frames shows the speed.
- Import: music (mp3, wav, m4a, flac…) on a lane of its own under the clip, with its block to drag, a volume and a fade at the end; pictures (png, jpg, webp) over the video on the "pictures" lane, dragged on the preview, each with its size and entrance. All of it goes into the mp4.
- Subtitles: a lane under the frames, T adds one at the playhead, drag its edges or the whole block. They are burnt into the mp4.
- Subtitle style: size, font (regular, bold, mono), CAPS, background (plate, outline, shadow), color, place. Drag the text up and down on the preview. The style is remembered.
- Subtitles from speech: "subtitles" → "recognize speech". Whisper runs on your computer, offline, nothing is uploaded; the language is detected. Two models: "quicker" (190 mb) or "more accurate" (570 mb), installed and removed in settings → system or with the button in the editor; nothing downloads on its own. It listens to the tracks that are on: mute the game to leave just the voice. A quiet mic is brought up to level; silence and made-up lines ("Subtitles by…") are dropped.

**Shots and clips**
- Text from the screen: "recognize text" in the panel after a selection (key T), and a key of its own that selects and reads at once (Ctrl+Alt+T by default, Ctrl+Opt+Cmd+T on a Mac). A window takes the area's place: each line lies over its own words on a dark plate, select and copy any of it, all of it is on the clipboard already. On Windows PaddleOCR (PP-OCRv5) reads it: Russian, Ukrainian, Belarusian and English mixed together, small text in dark interfaces too. The model is 13 mb and downloads once: the text window offers it the first time, and settings → system installs and removes it. It runs on your computer with the engine built into Windows; the picture goes nowhere. Vision reads it on a Mac. The window has a header with the line count and "copy all"; lines come in one by one, and each gets its own copy button on hover.
- Videos on blinker.cool links play right in Discord and Telegram: the page carries the video's size and a preview frame, and the server moves the mp4 index to the front so a messenger can play it before it has the whole file.
- QR codes on the screen: when you take a shot, a found code gets a frame with a sweeping line and a card under it saying where it leads, with an "open" button and one to copy the link (or "copy" when it isn't a link). On a Mac the card is glass, as is the recognized text window. Turn it off with "read qr codes" in settings → capture.
- Color picker: a key of its own (Ctrl+Alt+C by default, Ctrl+Opt+Cmd+C on a Mac). The screen freezes with the loupe at the pointer, a click copies the color as hex.
- Clips by game: recordings, replays and gifs made while a game runs are named after it ("Dota 2 2026-10-23 20-03-05.mp4") and go into "games / Dota 2". The video tab of the history filters by game. Turn it off in "storage". Windows for now.

**Logo**
- A new logo: a white "B" with a star cut out and a red dot. Classic on Windows, glass on a Mac (Liquid Glass on macOS 26).
- The tray and menu bar icon is a white star on a dark tile that reads even at 16 px. While the replay records, a red dot sits in its corner (the whole icon turned red before).
- The Discord status shows the new logo without the dot, so it doesn't clash with the recording badge in the corner.

**Other**
- After an automatic update blinker restarts quietly in the tray: the settings no longer open by themselves and pull you out of a full-screen game.
- "Settings" and "history" in the tray bring an open window to the front, minimized too.
- Build your own tray menu: settings → "tray menu". The menu is shown as it opens; drag items between the tiles on top, the list and the spare ones on the right, add rows of tiles, click a row's heading to rename it, put in separators; "reset" brings back the default one. "Read text" and the color picker can go in the menu now. The tray icon's clicks moved there too.
- The settings are laid out anew: "text and qr codes", "video editor" (opening it after a recording, subtitles from speech), "tray menu", "integrations" (Discord, ShareX, NVIDIA). The shutter sound is under "capture" now; "system" keeps the language, startup and updates.
- Esc no longer closes the history. A right click on the card dismisses it.
- Links to GitHub and to blinker's Discord server at the bottom of the settings (the invite opens in the Discord app when it's installed).
- blinker://update links: the "Update" button in blinker's Discord opens the settings and installs the new version right away (through blinker.cool/update; the browser asks to open blinker).
- Mac: a click takes a part of a window, as on Windows. It needs Accessibility: settings, "capture", the "allow" button.
- Mac: the recording pill of a whole-screen recording in "in the corner" sits bottom right above the Dock (it went to the top before).

## 0.4.0

**Снимки и запись**
- Клик выделяет часть окна: например, саму страницу в браузере без вкладок и адресной строки. Alt + клик берёт окно целиком.
- В записи клик по развёрнутому окну берёт весь экран, а не одно окно. В панели записи кнопка «весь экран» (то же, что пробел).
- Пока пишется весь экран, вокруг него нет рамки: из-за неё Windows считал запись полноэкранной программой, и панель задач с треем не поднималась при наведении. Плашка записи в этом случае стоит в углу, прямо над треем.
- Место плашки записи выбирается в «запись → плашка записи», отдельно для области и для всего экрана: на маленьких экранах с примером, кликом по месту. По умолчанию она под областью, а при записи всего экрана у трея.
- Если панель задач скрывается сама и выезжает поверх плашки, плашка плавно поднимается над ней и опускается обратно. Только если под плашкой правда кнопки, трей или часы: короткая «плавающая» панель плашку не трогает.
- В меню трея снимки и видео разделены, а у видео есть «экран»: запись всего экрана сразу, без выделения.

**История**
- Клик открывает снимок или видео в обычной программе, правый клик показывает справа от курсора небольшое меню со всеми действиями: править в blinker, копировать, загрузить, переименовать, показать в папке, в корзину.
- Карточку можно перетащить как файл: в чат, в папку, в браузер.
- Зажатое колёсико мыши прокручивает список.
- Вкладка «видео и gif».
- В загрузках крупное превью, клик по нему открывает файл; ссылка открывается кнопкой со значком ссылки.

**Discord**
- Отдельные галки: показывать игру и показывать статистику за день.
- Статистика считает снимки и клипы, как только они сделаны, а не файлы в папке: скопированное без сохранения тоже идёт в счёт.
- Строка под названием: «Screenshots & Clipping», подсказка на логотипе: «hewwo everynyan ;3».

**Прочее**
- С оверлеем NVIDIA настройки один раз объясняют, откуда «Press Alt+Z to open NVIDIA Overlay» и как её выключить; пошаговый гайд остаётся в «система → nvidia». Из программы её не убрать: NVIDIA считает игрой всё, что не вписала в свой список сама.
- Windows: настройки и история снова рисуются через OpenGL. Через DirectX оверлей NVIDIA принимал blinker за игру и каждый раз показывал «Press Alt+Z». Заодно blinker.exe стал на 4,5 мб легче.
- «Сжать для telegram» убрано: замеры на реальных записях показали, что файлы от него только росли (в 1,3–2,3 раза). blinker пишет только изменившиеся кадры, и записи уже лёгкие. Вместе с ним ушёл раздел ffmpeg в настройках.
- Новый значок play на видео в уведомлении и в истории.
- Путь к настройкам внизу раздела «система» открывает их папку.
- Переключатели, стоящие рядом (например, «видео / gif» и частота кадров в панели записи), больше не делят одну подложку.

---

**Capture and recording**
- A click takes a part of a window: a browser's page without its tabs and address bar, say. Alt + click takes the whole window. Windows for now.
- Recording, a click on a maximized window takes the whole screen, not the window alone. The record bar has a "whole screen" button (the same as Space).
- While a whole display is recorded there is no frame around it: Windows took it for a full-screen app, and the taskbar with the tray wouldn't come up on hover. The recording pill sits in the corner then, right above the tray.
- Choose where the recording pill sits in "recording → recording pill", for an area and for the whole screen apart: on small screens with an example, a click on the spot. By default it's under the area, and by the tray when the whole screen is recorded.
- A taskbar that hides itself and slides up over the pill: the pill glides above it and back down. Only when its buttons, tray or clock are really under the pill: a short "floating" taskbar leaves it be.
- The tray menu keeps screenshots and video apart, and video has "screen": the whole screen recorded at once, no selecting.

**History**
- A click opens a shot or a video in its usual app; a right click shows a small menu to the right of the pointer with every action: edit in blinker, copy, upload, rename, show in the folder, move to the bin.
- Drag a card out as a file: into a chat, a folder, a browser.
- Hold the mouse wheel down to drag the list.
- The "videos & gifs" tab.
- Uploads get a proper preview, a click on it opens the file; the link opens from a link button.

**Discord**
- Separate switches: show the game, show today's numbers.
- The numbers count shots and clips as they are made, not the files in the folder: copied-only shots count too.
- The line under the name: "Screenshots & Clipping", the logo's tooltip: "hewwo everynyan ;3".

**Other**
- With the NVIDIA overlay installed, the settings tell once where "Press Alt+Z to open NVIDIA Overlay" comes from and how to turn it off; the steps stay in "system → nvidia". An app can't quiet it: NVIDIA takes for a game anything it didn't put on its own list.
- Windows: the settings and the history draw with OpenGL again. Drawn with DirectX, they made the NVIDIA overlay take blinker for a game and say "Press Alt+Z" every time. blinker.exe is 4.5 mb lighter for it.
- "Compress for telegram" is gone: measured on real recordings, it only made files bigger (1.3–2.3 times). blinker writes only the frames that change, its recordings are light already. The ffmpeg section of the settings went with it.
- A new play mark on videos in the card and the history.
- The settings path at the bottom of "system" opens their folder.
- Choices side by side ("video / gif" and the frame rate in the record bar) no longer share one plate.

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
