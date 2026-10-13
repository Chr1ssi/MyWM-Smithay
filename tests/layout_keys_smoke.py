"""Bindings follow the configured keyboard layout: with `de`, the keys right of 0 and right of p are `ssharp` and
`udiaeresis`, named so in the config, whatever the X server's layout calls them.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/layout_keys_smoke.py
"""
import os, tempfile
from smoke_support import Compositor, wait

marks = tempfile.mkdtemp()
config = f"""[program_bindings.eszett]
keys = ["Super+ssharp"]
command = ["touch", "{marks}/ssharp"]

[program_bindings.umlaut]
keys = ["Super+udiaeresis"]
command = ["touch", "{marks}/udiaeresis"]
"""
comp = Compositor(config=config, layout="de")
try:
    # The X server is us: these are the keys at those places (nested, Alt stands for Super).
    comp.key("alt+minus")
    assert wait(lambda: os.path.exists(f"{marks}/ssharp"), 5), "Super+ssharp did nothing"
    comp.key("alt+bracketleft")
    assert wait(lambda: os.path.exists(f"{marks}/udiaeresis"), 5), "Super+udiaeresis did nothing"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
