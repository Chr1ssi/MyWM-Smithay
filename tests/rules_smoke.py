"""Window rules select by app id and title with `*` wildcards and place windows by output and fullscreen.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/rules_smoke.py
"""
import subprocess
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/splice-test-client"
RULES = """
[[rules]]
app_id = "splice.test.*"
title = "*Tresor*"
output = "virtual-1"

[[rules]]
title = "Spiel"
fullscreen = true

[[rules]]
app_id = "splice.test.windowed"
fullscreen = false
"""

comp = Compositor(config=RULES, extra_env={"SPLICE_VIRTUAL_OUTPUTS": "1"})
clients = []


def named(*args):
    client = subprocess.Popen([CLIENT, "named", "30", *args], env=comp.env(), stdout=subprocess.PIPE, text=True)
    clients.append(client)
    return client


def sizes(client, until):
    """`size` lines of the client until one satisfies `until`."""
    seen = []
    def check():
        line = client.stdout.readline().split()
        if line and line[0] == "size":
            seen.append((int(line[1]), int(line[2])))
        return seen and until(seen[-1])
    assert wait(check, 8), seen
    return seen[-1]


try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 2)
    first, second = sorted(bar.state, key=lambda o: o["x"])
    occupied = lambda output: lambda s: next(o for o in s if o["id"] == output["id"])["workspaces"].get(
        next(o for o in s if o["id"] == output["id"])["active"])

    # Title without "Tresor": no rule, the focused (first) monitor.
    named("splice.test.notes", "Notizen")
    bar.until(occupied(first))
    assert not occupied(second)(bar.state), bar.state
    # Both wildcards match: the rule's monitor, although the first one has the focus.
    named("splice.test.vault", "Bitwarden Tresor - Firefox")
    bar.until(occupied(second))
    print("app id and title wildcards, output OK")

    # The output's whole size, without gaps: fullscreen.
    size = sizes(named("other.app", "Spiel"), lambda s: s[0] > 0)
    assert size == (1280, 800), size
    # Asks for fullscreen itself, the rule keeps it windowed.
    size = sizes(named("splice.test.windowed", "-", "fullscreen"), lambda s: s[0] > 0)
    assert size != (1280, 800), size
    print("fullscreen rules OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
finally:
    for client in clients:
        client.kill()
print("OK")
