# dmgbuild settings for Blinker.dmg (scripts/macos-app.sh runs it): the app
# and an Applications shortcut on the background from dmg-background.swift.

import os.path

app = defines["app"]
assets = os.path.join(os.path.dirname(os.path.abspath(app)), "..", "..", "assets", "macos")

files = [app]
symlinks = {"Applications": "/Applications"}
hide_extensions = ["Blinker.app"]

format = "UDZO"
filesystem = "HFS+"
icon = os.path.join(assets, "blinker.icns")

# dmgbuild picks up dmg-background@2x.png next to it for Retina screens
background = os.path.join(assets, "dmg-background.png")
# the height takes in the title bar, the picture is the 660×400 under it
window_rect = ((200, 160), (660, 432))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False

icon_size = 128
text_size = 13
icon_locations = {
    "Blinker.app": (180, 200),
    "Applications": (480, 200),
}
