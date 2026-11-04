"""Exact native process start tokens for the pinned launcher's authenticated receipts.

Prepended by launch.rs and called before the terminal reader or registration retries.
Matches vw-procmon/vw-platform units without float timestamps or extra dependencies.
The terminal backend still owns the unreaped child / retained Windows process handle.
"""


def native_process_start_token(pid):
    import sys
    if not isinstance(pid, int) or not 0 < pid <= 0x7fffffff:
        raise OSError("Invalid background process pid")
    if sys.platform == "linux":
        # comm can contain spaces and parentheses; starttime is field 22.
        with open(f"/proc/{pid}/stat", "rb") as source:
            fields = source.read().rsplit(b")", 1)[1].split()
        return int(fields[19])
    if sys.platform == "win32":
        return _windows_process_start_token(pid)
    if sys.platform == "darwin":
        return _mac_process_start_token(pid)
    raise OSError("Background process identity is unavailable")


def _windows_process_start_token(pid):
    import ctypes
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    kernel.GetProcessTimes.restype = wintypes.BOOL
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    handle = kernel.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        created, exited, kernel_time, user = (wintypes.FILETIME() for _ in range(4))
        if not kernel.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited),
                                      ctypes.byref(kernel_time), ctypes.byref(user)):
            raise ctypes.WinError(ctypes.get_last_error())
        return (created.dwHighDateTime << 32) | created.dwLowDateTime
    finally:
        kernel.CloseHandle(handle)


def _mac_process_start_token(pid):
    import ctypes

    # sys/proc_info.h's proc_bsdinfo, also used by vw-platform::process_facts.
    class BsdInfo(ctypes.Structure):
        _fields_ = [(name, ctypes.c_uint32) for name in (
            "flags", "status", "xstatus", "pid", "ppid", "uid", "gid", "ruid",
            "rgid", "svuid", "svgid", "rfu_1",
        )] + [("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32)] + [
            (name, ctypes.c_uint32) for name in ("nfiles", "pgid", "pjobc", "tdev", "tpgid")
        ] + [("nice", ctypes.c_int32), ("start_sec", ctypes.c_uint64),
             ("start_usec", ctypes.c_uint64)]

    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    libproc.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                                    ctypes.c_void_p, ctypes.c_int]
    libproc.proc_pidinfo.restype = ctypes.c_int
    info = BsdInfo()
    size = ctypes.sizeof(info)
    if libproc.proc_pidinfo(pid, 3, 0, ctypes.byref(info), size) != size:  # PROC_PIDTBSDINFO
        raise OSError(ctypes.get_errno(), "Background process identity is unavailable")
    return info.start_sec * 1_000_000 + info.start_usec
