"""The settings GUI (mywm-settings) under Xvfb: capture a key, save, see the file change; keys are named as the
configured layout names them.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/settings_smoke.py
"""
import os, random, subprocess, time
from smoke_support import wait

# A short path of digits only, as the clicks below are positions: the sidebar shows the path at a fixed width,
# but letters differ in width and a random name must not move anything.
while True:
    work = f"/tmp/mywm-{random.randrange(10 ** 8):08d}"
    try:
        os.mkdir(work, 0o700)
        break
    except FileExistsError:
        pass
config = f"{work}/config.toml"
with open(config, "w") as f:
    f.write("# my settings\n[effects]\ncorner_radius = 12 # round\n")

# XDG_STATE_HOME: the window takes its colors from the saved theme, which must not be the user's.
env = dict(os.environ, XDG_CONFIG_HOME=work, XDG_STATE_HOME=work, MYWM_SOCKET=f"{work}/none.sock")
# On X only: with a Wayland session in the environment the window would open there, in the user's session.
for name in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET"):
    env.pop(name, None)
app = subprocess.Popen(["./target/debug/mywm-settings", "--config", config, "--page", "0"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def xdo(*args):
    # Keys are held for a moment: under software rendering a frame can take longer than a quick press and release.
    if args[0] == "key":
        args = ("key", "--delay", "100", *args[1:])
    subprocess.run(["xdotool", *args], check=True)


try:
    assert wait(lambda: subprocess.run(["xdotool", "search", "--name", "mywm Einstellungen"], capture_output=True).stdout.strip(), 40), "no window"
    window = subprocess.check_output(["xdotool", "search", "--name", "mywm Einstellungen"]).split()[0].decode()
    # The window exists before its first frame is drawn (slow under software rendering); input sent earlier is lost.
    def drawn():
        picture = subprocess.run(["import", "-window", window, "png:-"], capture_output=True).stdout
        count = subprocess.run(["convert", "png:-", "-format", "%k", "info:"], input=picture, capture_output=True).stdout
        return count.strip().isdigit() and int(count) > 10
    assert wait(drawn, 30), "the window was never drawn"
    xdo("windowfocus", window)

    # "+ Taste" of the first action (Konfiguration neu laden), then F5 with Super ticked by default.
    # The first inputs can get lost while the window is still starting: try a few times.
    for attempt in range(4):
        xdo("mousemove", "--window", window, "745", "136", "click", "1")
        time.sleep(1)
        xdo("key", "F5")
        time.sleep(1)
        xdo("key", "ctrl+s")
        if wait(lambda: "Super+F5" in open(config).read(), 4):
            break
    assert "Super+F5" in open(config).read(), open(config).read()
    text = open(config).read()
    print(text)
    # The key is added to the default binding; comments and other settings stay.
    assert '"Super+Shift+r"' in text and "# my settings" in text and "corner_radius = 12 # round" in text, text

    # A second, conflicting key is refused: Super+q belongs to "close"; adding it to reload marks a duplicate
    # and the save button stays disabled, so the file is unchanged.
    before = open(config).read()
    xdo("mousemove", "--window", window, "852", "136", "click", "1")
    time.sleep(0.7)
    xdo("key", "q")
    time.sleep(0.5)
    # The duplicate shows in the alert color (the click did reach "+ Taste"), not only in a file that stays as it was.
    def marked():
        picture = subprocess.run(["import", "-window", window, "png:-"], capture_output=True).stdout
        mean = subprocess.run(["convert", "png:-", "-fuzz", "12%", "-fill", "black", "+opaque", "#f38ba8", "-format", "%[fx:mean]", "info:"],
                              input=picture, capture_output=True).stdout
        return float(mean or 0) > 0
    assert wait(marked, 5), "the duplicate key was not marked"
    xdo("key", "ctrl+s")
    time.sleep(1)
    assert open(config).read() == before, "a duplicate binding was saved"
    print("duplicates OK")
except BaseException:
    app.kill()
    print(app.stderr.read().decode()[-1500:])
    raise
finally:
    app.kill()

# With a German layout, the key at the US place of [ is ü: the binding names it so, as the compositor sees it.
app.wait(5)
assert wait(lambda: not subprocess.run(["xdotool", "search", "--name", "mywm Einstellungen"], capture_output=True).stdout.strip()), "the first window stayed"
with open(config, "w") as f:
    f.write('[keyboard]\nlayout = "de"\n')
app = subprocess.Popen(["./target/debug/mywm-settings", "--config", config, "--page", "0"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
try:
    assert wait(lambda: subprocess.run(["xdotool", "search", "--name", "mywm Einstellungen"], capture_output=True).stdout.strip(), 40), "no window"
    window = subprocess.check_output(["xdotool", "search", "--name", "mywm Einstellungen"]).split()[0].decode()
    assert wait(drawn, 30), "the window was never drawn"
    xdo("windowfocus", window)
    for attempt in range(4):
        xdo("mousemove", "--window", window, "745", "136", "click", "1")
        time.sleep(1)
        # The X server's layout is us: this is the key right of p.
        xdo("key", "bracketleft")
        time.sleep(1)
        xdo("key", "ctrl+s")
        if wait(lambda: "Super+udiaeresis" in open(config).read(), 4):
            break
    assert "Super+udiaeresis" in open(config).read(), open(config).read()
    print("layout OK")
    print("OK")
except BaseException:
    app.kill()
    print(app.stderr.read().decode()[-1500:])
    raise
finally:
    app.kill()
