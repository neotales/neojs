#!/usr/bin/env python3
"""Verify that a git working tree matches a commit's tree, read-only.

Usage:
    verify_git_tree.py DIR REV [--allow-untracked PREFIX]... [--home HOME]

Unlike `git diff-index`, this never consults or refreshes the index, so it is
safe on a read-only checkout. It compares every tracked file's bytes and mode
against the blobs in REV, rejects gitlinks (submodules), rejects extra untracked
paths unless they match an allowed prefix, and rejects symlinks that resolve
outside DIR. Exit 0 clean, 2 mismatch, 1 error.
"""

import argparse
import os
import subprocess
import sys

GIT_ARGS = [
    "git", "--no-optional-locks",
    "-c", "credential.helper=",
    "-c", "core.askPass=",
    "-c", "http.followRedirects=false",
    "-c", "http.proxy=",
    "-c", "https.proxy=",
    "-c", "http.sslVerify=true",
    "-c", "protocol.version=2",
]


class Mismatch(Exception):
    pass


def git_env(home):
    return {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home,
        "XDG_CONFIG_HOME": os.path.join(home, ".config"),
        "LC_ALL": "C",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "/bin/false",
        "SSH_ASKPASS": "/bin/false",
        "GIT_SSH_COMMAND": "false",
        "GIT_ALLOW_PROTOCOL": "https",
        "GIT_PROTOCOL_FROM_USER": "0",
        "GIT_OPTIONAL_LOCKS": "0",
    }


def run_git(cwd, env, *args, check=True):
    proc = subprocess.run(
        GIT_ARGS + list(args), cwd=cwd, env=env,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    if check and proc.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed: {proc.stderr.decode(errors='replace').strip()}"
        )
    return proc


def parse_tree(raw):
    entries = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        meta, path = item.split(b"\t", 1)
        mode, kind, sha = meta.split(b" ")
        if kind != b"blob":
            raise Mismatch(f"unsupported tree entry {kind!r} for {path!r}")
        if mode == b"160000":
            raise Mismatch(f"gitlink/submodule not allowed: {path!r}")
        entries.append((mode.decode(), sha.decode(), os.fsdecode(path)))
    return entries


def stream_matches(path, size, stream, what):
    if not os.path.lexists(path):
        raise Mismatch(f"{what}: missing file")
    if os.path.islink(path):
        raise Mismatch(f"{what}: expected a regular file, found a symlink")
    if not os.path.isfile(path):
        raise Mismatch(f"{what}: not a regular file")
    with open(path, "rb") as handle:
        remaining = size
        while remaining:
            chunk = stream.read(min(remaining, 1 << 20))
            if not chunk:
                raise Mismatch(f"{what}: file shorter than blob")
            if handle.read(len(chunk)) != chunk:
                raise Mismatch(f"{what}: content differs")
            remaining -= len(chunk)
        if handle.read(1):
            raise Mismatch(f"{what}: file longer than blob")


def check_symlinks(root):
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        if ".git" in dirnames:
            dirnames.remove(".git")
        for entry in dirnames + filenames:
            path = os.path.join(dirpath, entry)
            if not os.path.islink(path):
                continue
            target = os.readlink(path)
            if os.path.isabs(target):
                raise Mismatch(f"absolute symlink: {path} -> {target}")
            resolved = os.path.realpath(os.path.join(dirpath, target))
            real_root = os.path.realpath(root)
            if resolved != real_root and not resolved.startswith(real_root + os.sep):
                raise Mismatch(f"escaping symlink: {path} -> {target}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory")
    parser.add_argument("revision")
    parser.add_argument("--allow-untracked", action="append", default=[])
    parser.add_argument("--home", default="/nonexistent")
    args = parser.parse_args()
    root = os.path.abspath(args.directory)
    env = git_env(args.home)

    try:
        raw = run_git(root, env, "ls-tree", "-r", "-z", args.revision).stdout
        entries = parse_tree(raw)

        proc = subprocess.Popen(
            GIT_ARGS + ["cat-file", "--batch"], cwd=root, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        assert proc.stdin and proc.stdout
        try:
            for mode, sha, path in entries:
                full = os.path.join(root, path)
                proc.stdin.write(sha.encode() + b"\n")
                proc.stdin.flush()
                header = proc.stdout.readline()
                parts = header.split()
                if len(parts) != 3 or parts[1] != b"blob":
                    raise Mismatch(f"cannot read blob {sha} for {path!r}")
                size = int(parts[2])
                if mode == "120000":
                    target = proc.stdout.read(size)
                    proc.stdout.read(1)
                    if not os.path.islink(full):
                        raise Mismatch(f"symlink expected: {path}")
                    if os.fsencode(os.readlink(full)) != target:
                        raise Mismatch(f"symlink target differs: {path}")
                else:
                    stream_matches(full, size, proc.stdout, path)
                    proc.stdout.read(1)
                    if mode not in ("100644", "100755"):
                        raise Mismatch(f"unsupported file mode {mode} for {path!r}")
                    executable = bool(os.stat(full).st_mode & 0o100)
                    if executable != (mode == "100755"):
                        raise Mismatch(f"executable bit differs: {path}")
        finally:
            proc.stdin.close()
            proc.wait()

        untracked = run_git(
            root, env, "ls-files", "--others", "--exclude-standard", "-z"
        ).stdout
        for item in untracked.split(b"\0"):
            if not item:
                continue
            path = os.fsdecode(item)
            if any(path == p or path.startswith(p.rstrip("/") + "/") for p in args.allow_untracked):
                continue
            raise Mismatch(f"unexpected untracked file: {path}")

        check_symlinks(root)
    except Mismatch as exc:
        print(f"verify_git_tree: {exc}", file=sys.stderr)
        return 2
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"verify_git_tree: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
