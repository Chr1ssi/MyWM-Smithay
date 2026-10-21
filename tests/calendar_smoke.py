"""The built-in shell's calendar panel against a stand-in for the mywm-calendar helper (and for foot): opened by the
clock, it asks for the month's events and the calendars; paging asks for the next month; the form checks its fields,
creates an event on the selected day (typed into its fields, in the calendar chosen from its list) and reloads; the
button opens the calendar app and closes the panel. Clicked at the places of the Quickshell panel on 1280x800.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/calendar_smoke.py
"""
import datetime, os, shutil, subprocess, sys, tempfile, time
from smoke_support import Compositor, wait

if not shutil.which("grim"):
    print("SKIP: grim is not installed")
    sys.exit(0)

CLOCK = (640, 22)
CONTENT_X = 460  # the panel's content: the panel in the middle, 400 wide, 20 in
PREVIOUS_MONTH = (CONTENT_X + 12, 120)
NEXT_MONTH = (CONTENT_X + 305 + 12, 120)
ADD = (736, 433)
TITLE_FIELD = (CONTENT_X + 100, 472)
CHOICE = (CONTENT_X + 100, 568)
SECOND_CALENDAR = (CONTENT_X + 100, 596)
CREATE = (CONTENT_X + 180, 601)
# On a day without events (the first of the month, after paging back), the card of an event is only a line of text,
# so the button is 33 higher than on a day with one (DejaVu Sans, as in the nested session).
OPEN_APP = (CONTENT_X + 180, 676)

today = datetime.date.today()
later = today + datetime.timedelta(days=3)
work = tempfile.mkdtemp()
stubs = f"{work}/bin"
os.makedirs(stubs)
record = f"{work}/record"
open(record, "w").close()
events = ('[{"title":"Zahnarzt","location":"Praxis","calendar":"Privat","startDate":"%s","endDate":"%s","startTime":"09:00",'
          '"endTime":"10:00","allDay":false,"color":"#ff8800"},{"title":"Urlaub","location":"","calendar":"Arbeit","startDate":"%s",'
          '"endDate":"%s","startTime":"","endTime":"","allDay":true,"color":""}]') % (today, today, later, later)


def stub(name, script):
    with open(f"{stubs}/{name}", "w") as f:
        f.write("#!/usr/bin/env bash\n" + script)
    os.chmod(f"{stubs}/{name}", 0o755)


# Every call is recorded with its arguments, one per line (an empty location as a trailing "|").
stub("mywm-calendar", f"""(IFS='|'; echo "$*") >> {record}
case $1 in
  events) echo '{events}';;
  calendars) echo '[{{"name":"Arbeit","color":""}},{{"name":"Privat","color":"#ff8800"}}]';;
esac
""")
stub("foot", f'(IFS="|"; echo "foot|$*") >> {record}\n')


def recorded():
    return open(record).read().splitlines()


def click(x, y):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)
    time.sleep(0.4)


def type_text(comp, text):
    window = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    subprocess.run(["xdotool", "windowfocus", window], check=True)
    subprocess.run(["xdotool", "type", "--delay", "80", text], check=True)
    time.sleep(0.3)


def pixel(comp, x, y):
    return subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:]


def first_of(year, month):
    return datetime.date(year + (month - 1) // 12, (month - 1) % 12 + 1, 1)


comp = None
try:
    extra = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent", "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
             "PIPEWIRE_RUNTIME_DIR": "/nonexistent", "PATH": f"{stubs}:{os.environ['PATH']}", "LANG": "de_DE.UTF-8"}
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    assert wait(lambda: pixel(comp, 15, 22) == bytes.fromhex("313244"), 20), "the bar was not drawn"

    # The clock opens the panel on today's month: its events up to a week into the next one, and the calendars.
    click(*CLOCK)
    month = first_of(today.year, today.month)
    following = first_of(today.year, today.month + 1)
    asked = f"events|{month}|{following.replace(day=8)}"
    assert wait(lambda: asked in recorded() and "calendars" in recorded()), recorded()
    # The form wants a title: without one nothing is created.
    click(*ADD)
    click(*CREATE)
    time.sleep(0.5)
    assert not any(line.startswith("create") for line in recorded()), recorded()
    # Typed into the title, created on the selected day (today) in the first calendar; the panel reloads.
    click(*TITLE_FIELD)
    type_text(comp, "Probe")
    click(*CREATE)
    assert wait(lambda: f"create|Arbeit|{today}|09:00|10:00|Probe|" in recorded()), recorded()
    assert wait(lambda: recorded().count(asked) == 2), recorded()
    print("create OK")

    # Created, the form folded away and forgot the title: opened again, a calendar chosen from the list.
    click(*ADD)
    click(*TITLE_FIELD)
    type_text(comp, "Zweiter")
    click(*CHOICE)
    click(*SECOND_CALENDAR)
    click(*CREATE)
    assert wait(lambda: f"create|Privat|{today}|09:00|10:00|Zweiter|" in recorded()), recorded()
    assert wait(lambda: recorded().count(asked) == 3), recorded()
    print("calendar choice OK")

    # The next month and back: their events, with their first day selected.
    click(*NEXT_MONTH)
    after = first_of(today.year, today.month + 2)
    assert wait(lambda: f"events|{following}|{after.replace(day=8)}" in recorded()), recorded()
    click(*PREVIOUS_MONTH)
    assert wait(lambda: recorded().count(asked) == 4), recorded()
    print("paging OK")

    # The app opens in foot; the panel closes (the clock is no longer selected).
    selected = pixel(comp, CLOCK[0], 9)  # in the clock's pill, above its text
    click(*OPEN_APP)
    assert wait(lambda: "foot|--app-id|mywm-calendar|--title|Kalender|mywm-calendar|app" in recorded()), recorded()
    assert wait(lambda: pixel(comp, CLOCK[0], 9) != selected), "the panel stayed open"
    print("app OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    print("recorded:", recorded())
    raise
