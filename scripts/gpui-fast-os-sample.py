#!/usr/bin/env python3
"""Sample macOS per-process rusage for one supplied PID; never signals it."""

import argparse
import ctypes
import ctypes.util
import datetime
import errno
import json
import os
from pathlib import Path
import platform
import tempfile
import time


RUSAGE_INFO_V0 = 0


class RUsageInfoV0(ctypes.Structure):
    _fields_ = [
        ("uuid", ctypes.c_ubyte * 16),
        ("user_time", ctypes.c_uint64),
        ("system_time", ctypes.c_uint64),
        ("pkg_idle_wkups", ctypes.c_uint64),
        ("interrupt_wkups", ctypes.c_uint64),
        ("pageins", ctypes.c_uint64),
        ("wired_size", ctypes.c_uint64),
        ("resident_size", ctypes.c_uint64),
        ("phys_footprint", ctypes.c_uint64),
        ("proc_start_abstime", ctypes.c_uint64),
        ("proc_exit_abstime", ctypes.c_uint64),
    ]


class MachTimebaseInfo(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


def load_proc_library():
    if platform.system() != "Darwin":
        raise RuntimeError("this sampler uses macOS libproc proc_pid_rusage")
    library = ctypes.util.find_library("proc") or "/usr/lib/libproc.dylib"
    libproc = ctypes.CDLL(library, use_errno=True)
    libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    libproc.proc_pid_rusage.restype = ctypes.c_int
    return libproc


def mach_timebase():
    """Return the kernel Mach absolute-time tick conversion to nanoseconds."""
    system = ctypes.CDLL(ctypes.util.find_library("System") or "/usr/lib/libSystem.B.dylib")
    system.mach_timebase_info.argtypes = [ctypes.POINTER(MachTimebaseInfo)]
    system.mach_timebase_info.restype = ctypes.c_int
    info = MachTimebaseInfo()
    result = system.mach_timebase_info(ctypes.byref(info))
    if result != 0 or info.denom == 0:
        raise RuntimeError(f"mach_timebase_info failed with kern_return_t {result}")
    return info


def read_rusage(libproc, pid):
    usage = RUsageInfoV0()
    result = libproc.proc_pid_rusage(pid, RUSAGE_INFO_V0, ctypes.byref(usage))
    if result != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), f"pid {pid}")
    return usage


def atomic_write(path, payload):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(payload, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_name, path)
    except BaseException:
        try:
            os.unlink(temp_name)
        except OSError:
            pass
        raise


def sample(pid, output, interval=1.0, max_samples=None):
    if pid <= 0:
        raise ValueError("pid must be a positive integer")
    if output.exists():
        raise FileExistsError(f"refusing to overwrite existing output: {output}")
    libproc = load_proc_library()
    timebase = mach_timebase()
    first = read_rusage(libproc, pid)
    identity = {
        "uuid": bytes(first.uuid).hex(),
        "procStartAbstime": int(first.proc_start_abstime),
    }
    payload = {
        "schemaVersion": 2,
        "source": "macOS libproc proc_pid_rusage RUSAGE_INFO_V0",
        "cpuTimeUnit": "mach_absolute_time_ticks",
        "machTimebase": {"numer": int(timebase.numer), "denom": int(timebase.denom),
                         "nanosecondsPerTick": timebase.numer / timebase.denom},
        "pid": pid,
        "processIdentity": identity,
        "sampleIntervalSeconds": interval,
        "startedAtUtc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "complete": False,
        "exitObserved": False,
        "samples": [],
    }
    count = 0
    atomic_write(output, payload)
    while max_samples is None or count < max_samples:
        try:
            usage = read_rusage(libproc, pid)
        except OSError as error:
            if error.errno != errno.ESRCH:
                raise
            payload["exitObserved"] = True
            break
        current_identity = {
            "uuid": bytes(usage.uuid).hex(),
            "procStartAbstime": int(usage.proc_start_abstime),
        }
        if current_identity != identity:
            payload["identityChanged"] = current_identity
            payload["complete"] = False
            atomic_write(output, payload)
            raise RuntimeError(f"PID {pid} identity changed during sampling; result rejected")
        payload["samples"].append({
            "unixTimeNs": time.time_ns(),
            "monotonicTimeNs": time.monotonic_ns(),
            "userTimeTicks": int(usage.user_time),
            "systemTimeTicks": int(usage.system_time),
            "packageIdleWakeups": int(usage.pkg_idle_wkups),
            "interruptWakeups": int(usage.interrupt_wkups),
            "pageIns": int(usage.pageins),
            "wiredBytes": int(usage.wired_size),
            "residentBytes": int(usage.resident_size),
            "physicalFootprintBytes": int(usage.phys_footprint),
        })
        count += 1
        payload["sampleCount"] = count
        atomic_write(output, payload)
        if usage.proc_exit_abstime != 0:
            payload["exitObserved"] = True
            break
        if max_samples is not None and count >= max_samples:
            break
        time.sleep(interval)
    payload["complete"] = True
    payload["finishedAtUtc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    payload["sampleCount"] = count
    atomic_write(output, payload)
    return payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", type=int, help="exact fixture PID printed by gpui-performance-session.py")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--interval", type=float, default=1.0)
    parser.add_argument("--max-samples", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.interval <= 0 or args.interval > 60:
        parser.error("interval must be greater than 0 and at most 60 seconds")
    if args.max_samples is not None and args.max_samples < 1:
        parser.error("max-samples must be positive")
    try:
        payload = sample(args.pid, args.output.resolve(), args.interval, args.max_samples)
    except (OSError, RuntimeError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps({"pid": payload["pid"], "samples": payload["sampleCount"],
                      "output": str(args.output), "identity": payload["processIdentity"]}))


if __name__ == "__main__":
    main()
