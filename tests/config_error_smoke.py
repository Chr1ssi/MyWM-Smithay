"""A broken configuration file neither keeps the session nor the lock screen from starting, and the bar learns why.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/config_error_smoke.py
"""
import subprocess
from smoke_support import Bar, Compositor, wait

BROKEN = "[appearance]\ngaps_inner = 'wide'\n"
comp = Compositor(config=BROKEN)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    assert wait(lambda: (bar.poll(), bar.config_error)[1]), "the bar was not told about the broken file"
    print("started with the defaults, bar told:", bar.config_error)
    assert "gaps_inner" in bar.config_error, bar.config_error

    # Fixed and reloaded: the message goes away.
    with open(f"{comp.dir}/config.toml", "w") as f:
        f.write(f'wallpaper_directory = "{comp.dir}/wallpapers"\n[keyboard]\nlayout = "us"\n[appearance]\ngaps_inner = 8\n')
    assert bar.send("theme-reload") == "v1 ok"
    assert wait(lambda: (bar.poll(), bar.config_error is None)[1]), bar.config_error
    print("fixed file clears the message OK")

    # Broken again on reload: the running configuration stays, the bar is told again.
    with open(f"{comp.dir}/config.toml", "a") as f:
        f.write("[bindings]\nnot_a_binding = ['Super+x']\n")
    assert bar.send("theme-reload") == "v1 ok"
    assert wait(lambda: (bar.poll(), bar.config_error)[1]), "a failed reload was not reported"
    print("failed reload reported:", bar.config_error)

    # The lock screen reads the file too (its colors); swayidle and Super+Escape depend on it. Last: the
    # session stays locked (ext-session-lock), and a locked compositor refuses most commands.
    locker = subprocess.run(["./target/debug/mywm-compositor", "--lock"], env=comp.env(), timeout=15)
    assert locker.returncode == 0, "--lock failed with a broken configuration"
    bar.until(lambda s: bar.locked, 5)
    print("locking works with a broken configuration OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
