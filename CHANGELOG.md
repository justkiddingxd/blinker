# Changelog

Each version's section becomes its release notes on GitHub (`.github/workflows/release.yml` takes the part under `## <version>`).

## 0.3.0

**Загрузка**
- Сервер по умолчанию — **post.blinker.cool**: без регистрации, файлы живут 3 дня, видео до 100 мб. У каждой ссылки своя страница: просмотр картинки, плеер видео и звука.
- Свой такой же сервер поднимается одной командой: `docker compose up` в папке `server` ([инструкция](https://github.com/justkiddingxd/blinker/blob/main/server/README.ru.md)).
- Загрузчики ShareX: подключи файл `.sxcu`, и снимки уходят на твой Zipline, XBackBone и другие.

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
- ShareX uploaders: pick an `.sxcu` file and captures go to your Zipline, XBackBone and the like.

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
