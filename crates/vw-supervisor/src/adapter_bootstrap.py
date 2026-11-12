"""Load the supervisor's private adapter; execute only the bytes whose digest it supplied."""
import hashlib
import sys

path, digest = sys.argv[1:3]
with open(path, "rb") as stream:
    source = stream.read()
if hashlib.sha256(source).hexdigest() != digest:
    raise SystemExit("Vibe launch adapter integrity check failed")
sys.argv = [path, *sys.argv[3:]]
exec(compile(source, path, "exec"), {"__name__": "__main__", "__file__": path})
