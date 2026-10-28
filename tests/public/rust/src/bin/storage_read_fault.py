"""Hold one real file read, without replacing Vibe or any protocol response.

Called by real_process_fault before forwarding a resume. Unix uses a FIFO with the
original bytes; Windows holds an NTFS oplock. EOF releases either fault early.
"""
import ctypes
import os
from pathlib import Path
import sys
import threading

path = Path(sys.argv[1])
delay = int(sys.argv[2]) / 1000
release = threading.Event()


def on_eof():
    sys.stdin.buffer.read()
    release.set()


threading.Thread(target=on_eof, daemon=True).start()
if os.name == "nt":
    from ctypes import wintypes as w

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)

    class Overlapped(ctypes.Structure):
        _fields_ = [("internal", ctypes.c_size_t), ("internal_high", ctypes.c_size_t),
                    ("offset", w.DWORD), ("offset_high", w.DWORD), ("event", w.HANDLE)]

    kernel.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, ctypes.c_void_p,
                                   w.DWORD, w.DWORD, w.HANDLE]
    kernel.CreateFileW.restype = w.HANDLE
    kernel.CreateEventW.argtypes = [ctypes.c_void_p, w.BOOL, w.BOOL, w.LPCWSTR]
    kernel.CreateEventW.restype = w.HANDLE
    kernel.DeviceIoControl.argtypes = [w.HANDLE, w.DWORD, ctypes.c_void_p, w.DWORD,
                                       ctypes.c_void_p, w.DWORD, ctypes.POINTER(w.DWORD),
                                       ctypes.POINTER(Overlapped)]
    kernel.CloseHandle.argtypes = [w.HANDLE]
    event = kernel.CreateEventW(None, True, False, None)
    handle = kernel.CreateFileW(str(path), 0x80000000, 7, None, 3, 0x40000000, None)
    if handle == w.HANDLE(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    overlap = Overlapped(event=event)
    returned = w.DWORD()
    try:
        # FSCTL_REQUEST_OPLOCK_LEVEL_1: competing opens wait until the holder closes.
        ok = kernel.DeviceIoControl(handle, 0x90000, None, 0, None, 0,
                                     ctypes.byref(returned), ctypes.byref(overlap))
        if not ok and ctypes.get_last_error() != 997:  # ERROR_IO_PENDING = oplock granted
            raise ctypes.WinError(ctypes.get_last_error())
        print("READY", flush=True)
        release.wait(delay)
    finally:
        kernel.CloseHandle(handle)
        kernel.CloseHandle(event)
else:
    import time

    original = path.read_bytes()
    saved = path.with_name(path.name + ".read-fault-original")
    path.rename(saved)
    try:
        os.mkfifo(path, 0o600)
        print("READY", flush=True)
        release.wait(delay)
        # A cancelled channel can still have a worker in read_bytes. Supply the real
        # file bytes to that worker before restoring the ordinary file.
        deadline = time.monotonic() + 1
        while True:
            try:
                fd = os.open(path, os.O_WRONLY | os.O_NONBLOCK)
                break
            except OSError as error:
                if error.errno != 6 or time.monotonic() >= deadline:
                    if error.errno == 6:
                        fd = None
                        break
                    raise
                time.sleep(0.001)
        if fd is not None:
            with os.fdopen(fd, "wb", buffering=0) as stream:
                stream.write(original)
    finally:
        path.unlink(missing_ok=True)
        saved.rename(path)
