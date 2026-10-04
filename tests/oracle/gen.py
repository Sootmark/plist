#!/usr/bin/env python3
"""Records what Python's plistlib reads in every fixture, the oracle the
integration tests compare this crate's reading with.

Run from anywhere (Python 3.13 or later): python3 tests/oracle/gen.py

Writes tests/oracle/<directory>/<fixture>.json for each fixture under
tests/fixtures/{generated,authored,plaso}, every value typed so nothing is
lost in JSON:

    {"dict": [[key, value], ...]}   file order kept
    {"array": [value, ...]}
    {"string": text}
    {"integer": "decimal digits"}   as text: up to 128 bits
    {"real": "16 hex digits"}       the IEEE 754 double's bits
    {"bool": true}
    {"date": "YYYY-MM-DDTHH:MM:SS.ffffffZ"}
    {"data": "hex"}
    {"uid": n}
    {"null": null}

Where plistlib and Apple disagree, Apple wins, and the dump follows it:
an XML dictionary holding only a CF$UID integer is a UID, as
CoreFoundation reads it. Fixtures plistlib can't read are listed in
UNREADABLE with the reason; the tests check them by hand. XML with
leading whitespace (which plistlib refuses to recognise) is read without
it, and control characters CoreFoundation writes raw into XML pass
through placeholders (see CONTROLS).
"""

import datetime
import json
import plistlib
import struct
from pathlib import Path

TESTS = Path(__file__).resolve().parent.parent
DIRECTORIES = ("generated", "authored", "plaso")
SUFFIXES = (".plist", ".bplist")

UNREADABLE = {
    # A date in year 0, which Python's datetime can't hold.
    "plaso/com.apple.security.KCN.plist",
    # Cut short: no trailer.
    "plaso/truncated.plist",
    # <plist> holding no value: plistlib returns None, Apple an error.
    "plaso/empty.plist",
}

# Control characters XML 1.0 forbids, which CoreFoundation writes raw in
# strings and expat refuses: read through private-use placeholders.
CONTROLS = [c for c in range(0x20) if c not in (0x09, 0x0A, 0x0D)]
PLACEHOLDER = 0xE000


def typed(value, xml):
    if isinstance(value, dict):
        if xml and list(value) == ["CF$UID"] and isinstance(value["CF$UID"], int):
            return {"uid": value["CF$UID"]}
        return {"dict": [[key, typed(item, xml)] for key, item in value.items()]}
    if isinstance(value, list):
        return {"array": [typed(item, xml) for item in value]}
    if isinstance(value, bool):
        return {"bool": value}
    if isinstance(value, int):
        return {"integer": str(value)}
    if isinstance(value, float):
        return {"real": struct.pack(">d", value).hex()}
    if isinstance(value, str):
        if xml:
            value = value.translate({PLACEHOLDER + c: c for c in CONTROLS})
        return {"string": value}
    if isinstance(value, bytes):
        return {"data": value.hex()}
    if isinstance(value, datetime.datetime):
        return {"date": value.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")}
    if isinstance(value, plistlib.UID):
        return {"uid": value.data}
    if value is None:
        return {"null": None}
    raise TypeError(f"unexpected {type(value)}")


def main():
    for directory in DIRECTORIES:
        out = TESTS / "oracle" / directory
        out.mkdir(parents=True, exist_ok=True)
        for old in out.glob("*.json"):
            old.unlink()
        for fixture in sorted((TESTS / "fixtures" / directory).iterdir()):
            name = f"{directory}/{fixture.name}"
            if fixture.suffix not in SUFFIXES or name in UNREADABLE:
                continue
            data = fixture.read_bytes()
            xml = not data.startswith(b"bplist")
            if xml:
                data = data.lstrip()
                for c in CONTROLS:
                    data = data.replace(bytes([c]), chr(PLACEHOLDER + c).encode())
            value = plistlib.loads(data, aware_datetime=True)
            dump = json.dumps(typed(value, xml), ensure_ascii=True, indent=1)
            (out / f"{fixture.name}.json").write_text(dump + "\n")


if __name__ == "__main__":
    main()
