#!/usr/bin/env python3
"""Writes the generated test property lists (tests/fixtures/generated) and
the binary copies of the authored ones (tests/fixtures/authored).

Run on macOS (Python 3.13 or later) from anywhere: python3 tests/fixtures/gen.py

Two writers make each file, so the reader meets both: Python's plistlib
(binary and XML) and Apple's CoreFoundation through plutil, which uniques
equal objects, sizes integers its own way and writes UIDs in XML as
CF$UID dictionaries. The keyed archives come from Foundation's
NSKeyedArchiver (archive.swift); Foundation orders their dictionaries by
hash, which changes from run to run, so they differ byte for byte each
time. Then tests/oracle/gen.py records what plistlib reads in every
fixture (run it after this script).
"""

import datetime
import plistlib
import subprocess
import tempfile
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent
GENERATED = FIXTURES / "generated"
AUTHORED = FIXTURES / "authored"

UTC = datetime.timezone.utc


def common_values():
    """Every type that both formats can hold."""
    return {
        "booleans": [True, False],
        "integers": [
            0, 1, 14, 15, 255, 256, 65_535, 65_536, 2**32 - 1, 2**32,
            2**63 - 1, 2**63, 2**64 - 1, -1, -255, -(2**31), -(2**63),
        ],
        "reals": [
            0.0, -0.0, 1.5, -2.25, 3.141592653589793, 1e300, 5e-324,
            float("inf"), float("-inf"), float("nan"),
        ],
        "dates": [
            datetime.datetime(2001, 1, 1, tzinfo=UTC),
            datetime.datetime(1970, 1, 1, tzinfo=UTC),
            datetime.datetime(1, 1, 1, tzinfo=UTC),
            datetime.datetime(1904, 2, 29, 12, 0, 1, tzinfo=UTC),
            datetime.datetime(2026, 10, 4, 18, 30, 15, 250_000, tzinfo=UTC),
            datetime.datetime(9999, 12, 31, 23, 59, 59, tzinfo=UTC),
        ],
        "data": [b"", b"\x00", bytes(range(14)), bytes(range(15)), bytes(range(256)) * 3],
        "strings": [
            "",
            "plain ASCII",
            "x" * 14,
            "y" * 15,
            "z" * 300,
            "café",
            "日本語のテキスト",
            "emoji \U0001f600 outside the BMP \U0001f9ea",
            "markup <tag> & \"quotes\" 'apostrophes' ]]> end",
            "  leading and trailing whitespace\n\ttabs  ",
        ],
        "empty array": [],
        "empty dict": {},
        "many": list(range(1000, 1040)),
        "wide": {f"key {index:03}": index for index in range(300)},
        "deep": nested(40),
        "unicode keys": {"clé": 1, "键": 2, "\U0001f511": 3},
        "order": {"zebra": 1, "apple": 2, "mango": 3},
    }


def nested(levels):
    value = "bottom"
    for level in range(levels):
        value = [value] if level % 2 else {"level": value}
    return value


def binary_only_values(wide_uid):
    """What XML can't hold: UIDs and control characters. CoreFoundation
    refuses UIDs over 32 bits, which plistlib writes: `wide_uid` adds one."""
    uids = [plistlib.UID(0), plistlib.UID(255), plistlib.UID(256), plistlib.UID(2**32 - 1)]
    return {
        "uids": uids + [plistlib.UID(2**32)] * wide_uid,
        "control characters": "nul \x00 bell \x07 escape \x1b",
    }


def write(path, value, fmt):
    path.write_bytes(plistlib.dumps(value, fmt=fmt, sort_keys=False, aware_datetime=True))


def plutil(source, target, fmt):
    subprocess.run(["plutil", "-convert", fmt, "-o", str(target), str(source)], check=True)


def main():
    GENERATED.mkdir(exist_ok=True)
    both = common_values()
    write(GENERATED / "types.xml.plist", both, plistlib.FMT_XML)
    write(GENERATED / "types.bplist", {**both, **binary_only_values(True)}, plistlib.FMT_BINARY)
    # CoreFoundation's writers, from plistlib's files.
    plutil(GENERATED / "types.xml.plist", GENERATED / "types.cf.bplist", "binary1")
    with tempfile.TemporaryDirectory() as scratch:
        source = Path(scratch) / "types.bplist"
        write(source, {**both, **binary_only_values(False)}, plistlib.FMT_BINARY)
        plutil(source, GENERATED / "types.cf.xml.plist", "xml1")
    # Over 255 objects (2-byte references) and over 64 KiB (3-byte offsets).
    large = {"blob": bytes(range(256)) * 300, "numbers": list(range(100_000, 100_400))}
    write(GENERATED / "large.bplist", large, plistlib.FMT_BINARY)
    subprocess.run(["swift", str(GENERATED / "archive.swift"), str(GENERATED)], check=True)
    for name in ("com.example.updater", "Info"):
        plutil(AUTHORED / f"{name}.plist", AUTHORED / f"{name}.bplist", "binary1")


if __name__ == "__main__":
    main()
