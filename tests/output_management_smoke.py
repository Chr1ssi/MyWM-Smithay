"""wlr-output-management with wlr-randr against the nested compositor (virtual outputs).

Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/output_management_smoke.py
"""
import subprocess
from smoke_support import Bar, Compositor, wait

comp = Compositor(windows=1, extra_env={"MYWM_VIRTUAL_OUTPUTS": "1"})
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 2)
    env = comp.env()

    def randr(*args):
        return subprocess.run(["wlr-randr", *args], env=env, capture_output=True, text=True, timeout=15)

    listing = randr()
    assert listing.returncode == 0, listing.stderr
    print(listing.stdout)
    names = [line.split()[0] for line in listing.stdout.splitlines() if line and not line[0].isspace()]
    assert len(names) == 2, names
    second = names[1]

    # Position and scale of the second output are applied and reach the bar.
    done = randr("--output", second, "--pos", "2000,100", "--scale", "2")
    assert done.returncode == 0, done.stderr
    bar.until(lambda s: s[1]["x"] == 2000, timeout=8)
    after = randr().stdout
    assert "Scale: 2.000000" in after, after
    assert "Position: 2000,100" in after, after

    # A mode the display does not have is refused.
    bad = randr("--output", second, "--mode", "123x45")
    assert bad.returncode != 0 or "123x45" not in randr().stdout

    # Switching an output off is not supported and leaves everything as it was.
    off = randr("--output", second, "--off")
    assert len(bar.state) == 2
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
