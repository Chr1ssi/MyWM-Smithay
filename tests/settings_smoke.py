"""The settings GUI (mywm-settings) under Xvfb: capture a key, save, see the file change.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/settings_smoke.py
"""
import os, subprocess, tempfile, time
from smoke_support import wait

work = tempfile.mkdtemp()
config = f"{work}/config.toml"
with open(config, "w") as f:
    f.write("# my settings\n[effects]\ncorner_radius = 12 # round\n")

env = dict(os.environ, XDG_CONFIG_HOME=work, MYWM_SOCKET=f"{work}/none.sock")
app = subprocess.Popen(["./target/debug/mywm-settings", "--config", config, "--page", "0"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def xdo(*args):
    subprocess.run(["xdotool", *args], check=True)


try:
    assert wait(lambda: subprocess.run(["xdotool", "search", "--name", "mywm Einstellungen"], capture_output=True).stdout.strip(), 15), "no window"
    window = subprocess.check_output(["xdotool", "search", "--name", "mywm Einstellungen"]).split()[0].decode()
    time.sleep(1.5)
    xdo("windowfocus", window)

    # "+ Taste" of the first action (Konfiguration neu laden), then F5 with Super ticked by default.
    xdo("mousemove", "--window", window, "295", "89", "click", "1")
    time.sleep(0.7)
    xdo("key", "F5")
    time.sleep(0.7)
    xdo("key", "ctrl+s")
    assert wait(lambda: "Super+F5" in open(config).read(), 8), open(config).read()
    text = open(config).read()
    print(text)
    # The key is added to the default binding; comments and other settings stay.
    assert '"Super+Shift+r"' in text and "# my settings" in text and "corner_radius = 12 # round" in text, text

    # A second, conflicting key is refused: Super+q belongs to "close"; adding it to reload marks a duplicate
    # and the save button stays disabled, so the file is unchanged.
    before = open(config).read()
    xdo("mousemove", "--window", window, "400", "89", "click", "1")
    time.sleep(0.7)
    xdo("key", "q")
    time.sleep(0.5)
    xdo("key", "ctrl+s")
    time.sleep(1)
    assert open(config).read() == before, "a duplicate binding was saved"
    print("OK")
finally:
    app.kill()
