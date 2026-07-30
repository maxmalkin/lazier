#!/usr/bin/env python3
"""Drive a TUI in a real 80x24 pty. Usage: tui.py KEYS TIMEOUT CMD [ARGS...]
Sends KEYS after 0.7s (constant for all tools -> comparisons stay fair),
waits for exit, prints RSS_MB= to stderr. Exit 124 on timeout."""
import os, pty, sys, time, fcntl, termios, struct, signal, select, resource

keys, timeout, cmd = sys.argv[1], float(sys.argv[2]), sys.argv[3:]
pid, fd = pty.fork()
if pid == 0:
    os.execvp(cmd[0], cmd)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
os.kill(pid, signal.SIGWINCH)  # in case the app read the size before our ioctl

# A '~' in KEYS splits it into bursts. Send one burst each 0.6s. The app can
# then load data between the bursts.
bursts = keys.split('~')
start, sent, code = time.time(), 0, 0
while True:
    if sent < len(bursts) and time.time() - start > 0.7 + 0.6 * sent:
        os.write(fd, bursts[sent].encode()); sent += 1
    if select.select([fd], [], [], 0.05)[0]:
        try:
            if not os.read(fd, 65536): pass
        except OSError:
            break  # EIO: child closed its end
    if os.waitpid(pid, os.WNOHANG)[0]:
        pid = 0; break
    if time.time() - start > timeout:
        os.kill(pid, signal.SIGKILL); code = 124; break
if pid: os.waitpid(pid, 0)
ru = resource.getrusage(resource.RUSAGE_CHILDREN)
print(f"RSS_MB={ru.ru_maxrss/1048576:.0f}", file=sys.stderr)
sys.exit(code)
