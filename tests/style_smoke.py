"""`[appearance] style = "cyberdeck"`: the shell's islands are chamfered instead of rounded, and the theme generated from
the wallpaper gets its stronger palette (surfaces sink toward black). Changing the style in the config and reloading
generates the theme again, in both directions.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/style_smoke.py
"""
import json, os, subprocess, tempfile, time
from smoke_support import Bar, Compositor, wait

images = tempfile.mkdtemp()
# A wallpaper with colors, so that there are accents to boost.
subprocess.run(["convert", "-size", "64x64", "gradient:#2a1a5c-#c0306a", f"{images}/wall.png"], check=True)
nowhere = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent", "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
           "PIPEWIRE_RUNTIME_DIR": "/nonexistent"}


def bar_rows(comp):
    """The first island's last painted pixel on its top edge (row 6) and in its middle (row 22), as the clear
    color is what lies outside it. Rounded corners pull the top edge in; chamfers cut only two corners."""
    shot = subprocess.run(["grim", "-g", "0,0 300x40", "-t", "ppm", "-"], env=comp.env(), capture_output=True,
                          check=True, timeout=15).stdout
    header, rest = shot.split(b"\n255\n", 1)
    width = int(header.split()[1])
    row = lambda y: [rest[(y * width + x) * 3:(y * width + x) * 3 + 3] for x in range(width)]
    background = row(0)[299]

    def last_painted(y):
        painted = [x for x, pixel in enumerate(row(y)[:120]) if pixel != background]
        return max(painted) if painted else -1

    return last_painted(6), last_painted(22)


def surface_luminance(state):
    hex_color = json.load(open(f"{state}/theme.json"))["colors"]["surface"]["default"]["hex"]
    channels = [int(hex_color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in channels]
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]


comp = None
try:
    comp = Compositor(top=f'builtin_shell = true\nwallpaper_directory = "{images}"\n', extra_env=nowhere)
    state = f"{comp.dir}/state/splice"
    config = f"{comp.dir}/config.toml"
    bar = Bar(comp.sock)

    def rounded():
        top, middle = bar_rows(comp)
        return middle > 0 and top < middle - 6

    def chamfered():
        top, middle = bar_rows(comp)
        return middle > 0 and top == middle

    assert wait(rounded, 20), bar_rows(comp)
    print("default style rounds the islands")

    # Switch to cyberdeck: the islands are cut, and the theme is generated again with the stronger palette.
    with open(config, "a") as f:
        f.write('\n[appearance]\nstyle = "cyberdeck"\n')
    assert bar.send("theme-reload") == "v1 ok"
    assert wait(lambda: os.path.exists(f"{state}/theme.json") and surface_luminance(state) < 0.0065, 30), \
        surface_luminance(state) if os.path.exists(f"{state}/theme.json") else "no theme"
    assert wait(chamfered, 20), bar_rows(comp)
    print("cyberdeck chamfers the islands and sinks the surfaces:", round(surface_luminance(state), 4))

    # And back.
    text = open(config).read().replace('style = "cyberdeck"', 'style = "default"')
    open(config, "w").write(text)
    assert bar.send("theme-reload") == "v1 ok"
    assert wait(lambda: surface_luminance(state) > 0.0065, 30), surface_luminance(state)
    assert wait(rounded, 20), bar_rows(comp)
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    raise
