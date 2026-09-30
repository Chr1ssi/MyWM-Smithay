"""Smoke test of the bar socket against the nested compositor.

Needs an X server ($DISPLAY, e.g. Xvfb), weston-simple-shm and a debug build
(`cargo build`). Run from the repository root: python3 tests/ipc_smoke.py
"""
import os, socket, subprocess, sys, tempfile, time
S=tempfile.mkdtemp(); SOCK=f"{S}/ctl.sock"
open(f"{S}/test.toml","w").write('[keyboard]\nlayout = "us"\n')
if os.path.exists(SOCK): os.unlink(SOCK)
env=dict(os.environ, MYWM_SOCKET=SOCK, MYWM_CONFIG=f"{S}/test.toml", RUST_LOG="info")
env.pop("WAYLAND_DISPLAY",None)
comp=subprocess.Popen(["./target/debug/mywm-compositor","sleep 1; for i in 1 2 3; do weston-simple-shm & sleep 0.3; done; wait"],env=env,stdout=open(f"{S}/ipc.log","w"),stderr=subprocess.STDOUT)
def wait(cond,t=8):
    end=time.time()+t
    while time.time()<end:
        if cond(): return True
        time.sleep(0.05)
    return False
assert wait(lambda: os.path.exists(SOCK)), "socket missing"
mode=os.stat(SOCK).st_mode & 0o777; print("socket mode",oct(mode)); assert mode==0o600
c=socket.socket(socket.AF_UNIX); c.connect(SOCK); c.settimeout(0.1)
buf=b""; snaps=[]; replies=[]
def poll():
    global buf
    try: buf+=c.recv(65536)
    except socket.timeout: pass
    *lines,buf=buf.split(b"\n")
    for l in lines:
        t=l.decode()
        if t.startswith("v1 state"): snaps.append(t)
        elif t.startswith("v1 ok") or t.startswith("v1 error"): replies.append(t)
def last(): poll(); return snaps[-1] if snaps else None
assert wait(lambda: last() and ",1:1" in last().split(" ",2)[2] ), f"no 3-window state: {last()}"
time.sleep(3); poll(); print("initial:",last())
def send(cmd):
    n=len(replies); c.sendall(f"v1 {cmd}\n".encode()); assert wait(lambda:(poll(),len(replies)>n)[1]); return replies[-1]
print(send("new-workspace 1"), last())
assert wait(lambda: "1,0,0,1280,800,2," in (last() or "")), last()
print(send("workspace 1 1"))
assert wait(lambda: (last() or "").split(" ")[2].split(",")[5]=="1"), last()
print("bad output:",send("workspace 9 1"), "bad ws:",send("workspace 1 7"), "garbage:",send("frobnicate"))
print(send("scratchpad"))  # empty scratchpad: valid, no change
# one-shot sender closing right after write still gets handled
o=socket.socket(socket.AF_UNIX); o.connect(SOCK); o.sendall(b"v1 new-workspace 1\n"); o.close()
assert wait(lambda: (last() or "").split(" ")[2].split(",")[5]=="2"), last()
print("final:",last())
comp.terminate(); comp.wait(5)
assert not os.path.exists(SOCK), "socket not removed on exit"
print("OK")
