"""Runs civ6opt in the background, detached from the terminal, so a long search keeps running when the terminal or
the session that started it closes. Works the same on Windows, macOS and Linux.

  job.py start <job> <civ6opt command and arguments>   start a job (returns at once)
  job.py status <job>                                  "start <time>", then "done exit=<code> <time>" when finished,
                                                       and the last lines of the job's stderr and stdout
  job.py list                                          every job and its last status line

A job's files are in data/jobs/<job>: status.txt, stdout.txt, stderr.txt and the command (command.json). The
arguments are passed as given, and relative paths are read from the folder job.py was started in.
"""
import json
import os
import subprocess
import sys
import time

import civ6opt

JOBS = civ6opt.ROOT / "data" / "jobs"


def now():
    return time.strftime("%Y-%m-%d %H:%M:%S")


def detached():
    """Keyword arguments of subprocess.Popen that start a process outside the terminal's process group (and, on
    Windows, without a console), so closing the terminal does not end it."""
    if os.name == "nt":
        return {"creationflags": subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW}
    return {"start_new_session": True}


def start(job, args):
    d = JOBS / job
    if d.exists():
        sys.exit(f"job {job} exists: {d}")
    d.mkdir(parents=True)
    (d / "command.json").write_text(json.dumps({"cwd": os.getcwd(), "args": args}))
    subprocess.Popen([sys.executable, __file__, "_run", job], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, cwd=os.getcwd(), **detached())
    print(f"started {job} in {d}")


def run(job):
    d = JOBS / job
    cmd = json.loads((d / "command.json").read_text())
    with open(d / "status.txt", "w") as s:
        print(f"start {now()}", file=s)
    with open(d / "stdout.txt", "w") as out, open(d / "stderr.txt", "w") as err:
        code = civ6opt.run(cmd["args"], cwd=cmd["cwd"], stdout=out, stderr=err).returncode
    with open(d / "status.txt", "a") as s:
        print(f"done exit={code} {now()}", file=s)


def tail(path, n):
    return path.read_text(errors="replace").splitlines()[-n:] if path.exists() else []


def status(job):
    d = JOBS / job
    print("\n".join(tail(d / "status.txt", 2) + tail(d / "stderr.txt", 3) + tail(d / "stdout.txt", 1)))


def main():
    a = sys.argv[1:]
    if len(a) >= 3 and a[0] == "start":
        start(a[1], a[2:])
    elif len(a) == 2 and a[0] == "status":
        status(a[1])
    elif len(a) == 2 and a[0] == "_run":
        run(a[1])
    elif a == ["list"]:
        for d in sorted(JOBS.iterdir()) if JOBS.exists() else []:
            print(d.name, "|", " ".join(tail(d / "status.txt", 1)))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
