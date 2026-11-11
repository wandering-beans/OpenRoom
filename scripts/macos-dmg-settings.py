# Finder coordinates are in points, including on Retina displays.
format = "UDZO"
files = [defines["app"]]
symlinks = {"Applications": "/Applications"}
background = defines["background"]
# Leave room for the Finder title bar above the 640 x 260 background.
window_rect = ((200, 200), (640, 292))
icon_size = 112
text_size = 14
grid_spacing = 80
icon_locations = {"OpenRoom.app": (154, 115), "Applications": (486, 115)}
