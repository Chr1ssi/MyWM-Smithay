"""The built-in shell's notifications (the service in splice-shell-services, toasts and the notification center) on a
private session bus: a toast shows and hides after its time unless the pointer holds it, its "×" and its action
answer the app over D-Bus, the app can close it, and the bell opens the notification center, which clears all,
closes with Escape and when something else is clicked.

Needs dbus-daemon, dbus-monitor, busctl and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/notifications_smoke.py
"""
import os, re, shutil, subprocess, sys, tempfile, time
from smoke_support import Compositor, private_bus, wait

for tool in ("dbus-daemon", "dbus-monitor", "busctl", "grim"):
    if not shutil.which(tool):
        print(f"SKIP: {tool} is not installed")
        sys.exit(0)

BACKGROUND = bytes.fromhex("1E1E2E")
WIDTH = 1280
TOAST = (WIDTH - 8 - 390, 46)  # the toast column's top left corner
PANEL = (WIDTH - 8 - 400, 46)  # the notification center's
BELL = (1170, 22)

work = tempfile.mkdtemp()
os.makedirs(f"{work}/session")
bus_command, address = private_bus(f"{work}/session")
processes = [subprocess.Popen(bus_command)]
assert wait(lambda: os.path.exists(f"{work}/session/bus")), "no bus"
env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address)
signals = open(f"{work}/signals.log", "w")
processes.append(subprocess.Popen(["dbus-monitor", "--address", address, "type='signal',interface='org.freedesktop.Notifications'"],
                                  stdout=signals, stderr=subprocess.STDOUT))


def call(method, signature="", *args):
    result = subprocess.run(["busctl", f"--address={address}", "call", "org.freedesktop.Notifications", "/org/freedesktop/Notifications",
                             "org.freedesktop.Notifications", method, *([signature] if signature else []), *args],
                            env=env, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    return result.stdout.strip()


def notify(summary, timeout, actions=(), body="", app=""):
    out = call("Notify", "susssasa{sv}i", app, "0", "", summary, body, str(len(actions)), *actions, "0", str(timeout))
    return int(out.split()[1])


def region(comp, x, y, w, h):
    shot = subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True, timeout=15).stdout
    return shot[-w * h * 3:]


def empty(comp, x, y, w, h):
    data = region(comp, x, y, w, h)
    return all(data[i:i + 3] == BACKGROUND for i in range(0, len(data), 3))


def toast_shows(comp):
    # The first toast's title.
    return not empty(comp, TOAST[0] + 12, TOAST[1] + 12, 120, 18)


def panel_shows(comp):
    # The center's title, "Benachrichtigungen".
    return not empty(comp, PANEL[0] + 20, PANEL[1] + 20, 150, 22)


def click(x, y):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)


def signalled(pattern):
    signals.flush()
    return re.search(pattern, open(f"{work}/signals.log").read(), re.S)


comp = None
try:
    extra = {"DBUS_SESSION_BUS_ADDRESS": address, "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
             "PIPEWIRE_RUNTIME_DIR": "/nonexistent", "LANG": "de_DE.UTF-8"}
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    assert wait(lambda: subprocess.run(["busctl", f"--address={address}", "status", "org.freedesktop.Notifications"],
                                       capture_output=True).returncode == 0, 15), "the notification service did not come"
    assert call("GetServerInformation").startswith('ssss "splice" "splice"'), call("GetServerInformation")
    assert '"actions"' in call("GetCapabilities") and '"body-markup"' in call("GetCapabilities")
    subprocess.run(["xdotool", "mousemove", "300", "500"], check=True)
    time.sleep(2)

    # A toast hides after its time; the notification stays behind the bell (the center lists it below).
    first = notify("Toast eins", 2000)
    assert wait(lambda: toast_shows(comp), 5), "no toast"
    shown = time.time()
    assert wait(lambda: not toast_shows(comp), 5), "the toast did not hide"
    assert 1.5 < time.time() - shown < 3.5, time.time() - shown
    print("toast hides after its time OK")

    # The pointer over a toast holds it; once it leaves, the time starts again.
    held = notify("Halten", 1500)
    assert wait(lambda: toast_shows(comp), 5)
    subprocess.run(["xdotool", "mousemove", str(TOAST[0] + 150), str(TOAST[1] + 30)], check=True)
    time.sleep(3)
    assert toast_shows(comp), "the held toast hid"
    subprocess.run(["xdotool", "mousemove", "300", "500"], check=True)
    assert wait(lambda: not toast_shows(comp), 4), "the toast did not hide after the pointer left"
    print("held toast OK")

    # "×" dismisses: the app hears reason 2.
    closing = notify("Schließen", 0)
    assert wait(lambda: toast_shows(comp), 5)
    click(TOAST[0] + 12 + (390 - 24 - 32) - 12, TOAST[1] + 12 + 13)
    assert wait(lambda: signalled(rf"NotificationClosed\s+uint32 {closing}\s+uint32 2"), 5), "no NotificationClosed for ×"
    assert wait(lambda: not toast_shows(comp), 5)
    print("dismiss OK")

    # An action: the app hears which, and the notification closes (it is not resident).
    acting = notify("Aktion", 0, actions=("default", "Öffnen"))
    assert wait(lambda: toast_shows(comp), 5)
    click(TOAST[0] + 12 + 20, TOAST[1] + 12 + 26 + 4 + 13)
    assert wait(lambda: signalled(rf'ActionInvoked\s+uint32 {acting}\s+string "default"'), 5), "no ActionInvoked"
    assert wait(lambda: signalled(rf"NotificationClosed\s+uint32 {acting}\s+uint32 2"), 5)
    assert wait(lambda: not toast_shows(comp), 5)
    print("action OK")

    # The app closes its own notification: reason 3.
    own = notify("Selbst", 0)
    assert wait(lambda: toast_shows(comp), 5)
    call("CloseNotification", "u", str(own))
    assert wait(lambda: signalled(rf"NotificationClosed\s+uint32 {own}\s+uint32 3"), 5)
    assert wait(lambda: not toast_shows(comp), 5)
    print("CloseNotification OK")

    # The bell opens the center (toasts hide meanwhile); "Alle löschen" clears all (the timed-out ones too).
    staying = notify("Bleibt", 0)
    assert wait(lambda: toast_shows(comp), 5)
    click(*BELL)
    assert wait(lambda: panel_shows(comp), 5), "the bell did not open the notification center"
    assert empty(comp, TOAST[0] + 12, TOAST[1] + 300, 120, 18)
    click(PANEL[0] + 400 - 20 - 50, PANEL[1] + 20 + 42 + 13)
    for id in (first, held, staying):
        assert wait(lambda: signalled(rf"NotificationClosed\s+uint32 {id}\s+uint32 2"), 5), f"{id} was not cleared"
    # The center says it is empty.
    assert wait(lambda: not empty(comp, PANEL[0] + 100, PANEL[1] + 20 + 42 + 26 + 12 + 30, 200, 20), 5)
    print("notification center clears OK")

    # Clicked into, Escape closes it; a click elsewhere does too.
    click(PANEL[0] + 200, PANEL[1] + 30)
    comp.key("Escape")
    assert wait(lambda: not panel_shows(comp), 5), "Escape did not close the notification center"
    click(*BELL)
    assert wait(lambda: panel_shows(comp), 5)
    click(300, 500)
    assert wait(lambda: not panel_shows(comp), 5), "a click elsewhere did not close the notification center"
    print("notification center closes OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    print(open(f"{work}/signals.log").read()[-2000:])
    raise
finally:
    for process in processes:
        process.kill()
