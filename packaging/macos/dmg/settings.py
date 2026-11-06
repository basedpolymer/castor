# dmgbuild settings for the Castor DMG (https://dmgbuild.readthedocs.io).
# Called by packaging/macos/common.sh:dmg_build with -D app=… -D background=… -D icon=…
import os.path

application = defines["app"]  # noqa: F821 (provided by dmgbuild)
appname = os.path.basename(application)

format = "ULFO"                 # LZFSE, macOS 10.11+
filesystem = "HFS+"
size = None                     # computed from the contents
files = [application]
symlinks = {"Applications": "/Applications"}
badge_icon = None
icon = defines.get("icon")      # noqa: F821  volume icon

background = defines["background"]  # noqa: F821  660x400 @1x/@2x TIFF (background.svg)
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
window_rect = ((200, 120), (660, 400))
default_view = "icon-view"
show_icon_preview = False
arrange_by = None
grid_offset = (0, 0)
grid_spacing = 100
label_pos = "bottom"
text_size = 13
icon_size = 112
icon_locations = {
    appname: (170, 190),
    "Applications": (490, 190),
}
