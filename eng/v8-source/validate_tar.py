#!/usr/bin/env python3
"""Validate, or validate-and-extract, a tar archive without trusting it.

Usage:
    validate_tar.py --check ARCHIVE
    validate_tar.py --extract ARCHIVE DEST

Extraction refuses absolute paths, `..` traversal, escaping symlinks and
hardlinks, device/fifo members, duplicate names, members whose ancestor is a
symlink or a file, and setuid/setgid bits. Modes are normalised: directories
0755, files 0644 or 0755 when any execute bit was set. Ownership and mtimes are
not preserved. DEST must be empty or absent.
"""

import argparse
import os
import posixpath
import shutil
import stat
import sys
import tarfile

REGULAR = {tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.CONTTYPE}
SYMLINK = tarfile.SYMTYPE
HARDLINK = tarfile.LNKTYPE
DIRECTORY = tarfile.DIRTYPE
ALLOWED = REGULAR | {SYMLINK, HARDLINK, DIRECTORY}


class Invalid(Exception):
    pass


def clean_name(name, what="member"):
    if name in ("", ".", "./"):
        raise Invalid(f"{what}: empty or dot name")
    if "\x00" in name or "\\" in name:
        raise Invalid(f"{what}: NUL or backslash in {name!r}")
    if name.startswith("/"):
        raise Invalid(f"{what}: absolute path {name!r}")
    if len(name) >= 2 and name[1] == ":" and name[0].isalpha():
        raise Invalid(f"{what}: drive-letter path {name!r}")
    stripped = name.rstrip("/")
    if not stripped:
        raise Invalid(f"{what}: empty path {name!r}")
    parts = stripped.split("/")
    for part in parts:
        if part in ("", ".", ".."):
            raise Invalid(f"{what}: unsafe component in {name!r}")
    return stripped


def inside(root, path):
    real_root = os.path.realpath(root)
    real_path = os.path.realpath(path)
    return real_path == real_root or real_path.startswith(real_root + os.sep)


def inspect(tf):
    members = tf.getmembers()
    if not members:
        raise Invalid("archive is empty")
    seen = {}
    kinds = {}
    for m in members:
        name = clean_name(m.name)
        if name in seen:
            raise Invalid(f"duplicate member {name!r}")
        if m.type not in ALLOWED:
            raise Invalid(f"member {name!r}: forbidden type {m.type!r}")
        if m.mode & 0o6000:
            raise Invalid(f"member {name!r}: setuid/setgid bit")
        seen[name] = m
        kinds[name] = (
            "dir" if m.isdir() else
            "link" if m.issym() else
            "hardlink" if m.islnk() else
            "file"
        )

    # No member may have a symlink or non-directory ancestor. Checked against
    # the archive's own name set, so archive order cannot matter.
    for name in seen:
        parts = name.split("/")
        for i in range(1, len(parts)):
            ancestor = "/".join(parts[:i])
            kind = kinds.get(ancestor)
            if kind == "link":
                raise Invalid(f"member {name!r}: ancestor {ancestor!r} is a symlink")
            if kind in ("file", "hardlink"):
                raise Invalid(f"member {name!r}: ancestor {ancestor!r} is a file")

    for name, m in seen.items():
        if m.issym():
            target = m.linkname
            if not target:
                raise Invalid(f"symlink {name!r}: empty target")
            if target.startswith("/") or "\x00" in target or "\\" in target:
                raise Invalid(f"symlink {name!r}: unsafe target {target!r}")
            if len(target) >= 2 and target[1] == ":" and target[0].isalpha():
                raise Invalid(f"symlink {name!r}: drive-letter target")
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
            if resolved == ".." or resolved.startswith("../") or resolved.startswith("/"):
                raise Invalid(f"symlink {name!r}: target escapes root ({target!r})")
        if m.islnk():
            target = clean_name(m.linkname, f"hardlink {name!r} target")
            if target not in seen:
                raise Invalid(f"hardlink {name!r}: target {target!r} is not in archive")
            if kinds[target] != "file":
                raise Invalid(f"hardlink {name!r}: target {target!r} is not a regular file")
    return members


def extract(tf, members, dest):
    os.makedirs(dest, mode=0o755, exist_ok=True)
    for m in members:
        name = clean_name(m.name)
        path = os.path.join(dest, *name.split("/"))
        parent = os.path.dirname(path)
        os.makedirs(parent, mode=0o755, exist_ok=True)
        if not inside(dest, parent):
            raise Invalid(f"member {name!r}: parent escapes root")
        if m.isdir():
            if os.path.lexists(path) and not os.path.isdir(path):
                raise Invalid(f"directory {name!r}: path already exists")
            os.makedirs(path, mode=0o755, exist_ok=True)
            os.chmod(path, 0o755)
            continue
        if os.path.lexists(path):
            raise Invalid(f"member {name!r}: path already exists")
        if m.issym():
            os.symlink(m.linkname, path)
            continue
        if m.islnk():
            target = clean_name(m.linkname, f"hardlink {name!r} target")
            target_path = os.path.join(dest, *target.split("/"))
            if not inside(dest, target_path) or not stat.S_ISREG(os.lstat(target_path).st_mode):
                raise Invalid(f"hardlink {name!r}: unsafe or non-regular target")
            os.link(target_path, path)
            continue
        mode = 0o755 if (m.mode & 0o100) else 0o644
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(fd, "wb") as out:
                src = tf.extractfile(m)
                if src is None:
                    raise Invalid(f"member {name!r}: unreadable")
                shutil.copyfileobj(src, out)
        finally:
            os.chmod(path, mode)

    # Defense in depth: every symlink must still resolve inside DEST.
    for dirpath, dirnames, filenames in os.walk(dest, followlinks=False):
        for entry in dirnames + filenames:
            path = os.path.join(dirpath, entry)
            if os.path.islink(path) and not inside(dest, path):
                raise Invalid(f"symlink escapes root after extraction: {path!r}")


def main():
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--check", action="store_true")
    group.add_argument("--extract", action="store_true")
    parser.add_argument("archive")
    parser.add_argument("dest", nargs="?")
    args = parser.parse_args()
    if args.extract and not args.dest:
        parser.error("--extract requires DEST")
    try:
        with tarfile.open(args.archive, "r:*") as tf:
            members = inspect(tf)
            if args.extract:
                if os.path.lexists(args.dest):
                    if not os.path.isdir(args.dest) or os.listdir(args.dest):
                        raise Invalid(f"destination is not empty: {args.dest}")
                extract(tf, members, args.dest)
    except (Invalid, tarfile.TarError, OSError) as exc:
        print(f"validate_tar: {exc}", file=sys.stderr)
        return 2 if isinstance(exc, Invalid) else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
