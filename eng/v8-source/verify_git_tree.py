#!/usr/bin/env python3
"""Verify that a git working tree matches a commit's tree, read-only.

Usage:
    verify_git_tree.py DIR REV [--allow-extra PREFIX]... [--home HOME]

Unlike `git diff-index`, this never consults or refreshes the index, so it is
safe on a read-only checkout. It compares every tracked file's bytes and mode
against the blobs in REV, rejects gitlinks (submodules), rejects every extra
path -- including ignored files and untracked directories -- unless it matches
an allowed prefix, and rejects symlinks that resolve outside DIR. Exit 0 clean,
2 mismatch, 1 error.
"""

import argparse
import os
import subprocess
import sys

GIT_ARGS = [
    "git", "--no-optional-locks",
    "-c", "credential.helper=",
    "-c", "core.askPass=",
    "-c", "core.hooksPath=/nonexistent",
    "-c", "core.fsmonitor=false",
    "-c", "core.autocrlf=false",
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
        try:
            meta, path = item.split(b"\t", 1)
            mode, kind, sha = meta.split(b" ")
        except ValueError as exc:
            raise Mismatch(f"malformed tree entry {item!r}") from exc
        if mode == b"160000":
            raise Mismatch(f"gitlink/submodule not allowed: {path!r}")
        if kind != b"blob":
            raise Mismatch(f"unsupported tree entry {kind!r} for {path!r}")
        entries.append((mode.decode(), sha.decode(), os.fsdecode(path)))
    return entries


class Batch:
    """A cat-file --batch process that is always terminated and reaped."""

    def __init__(self, cwd, env):
        self.proc = subprocess.Popen(
            GIT_ARGS + ["cat-file", "--batch"], cwd=cwd, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        if not self.proc.stdin or not self.proc.stdout:
            raise RuntimeError("cannot open cat-file pipes")

    def header(self, sha):
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(sha.encode() + b"\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        parts = line.split()
        if len(parts) != 3 or parts[1] != b"blob":
            raise Mismatch(f"cannot read blob {sha}")
        return int(parts[2])

    def read(self, size):
        assert self.proc.stdout
        return self.proc.stdout.read(size)

    def skip_newline(self):
        assert self.proc.stdout
        self.proc.stdout.read(1)

    def close(self):
        try:
            if self.proc.stdin:
                self.proc.stdin.close()
        except OSError:
            pass
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        for pipe in (self.proc.stdout, self.proc.stderr):
            try:
                if pipe:
                    pipe.close()
            except OSError:
                pass


def stream_matches(path, size, batch, what):
    if not os.path.lexists(path):
        raise Mismatch(f"{what}: missing file")
    if os.path.islink(path):
        raise Mismatch(f"{what}: expected a regular file, found a symlink")
    if not os.path.isfile(path):
        raise Mismatch(f"{what}: not a regular file")
    with open(path, "rb") as handle:
        remaining = size
        while remaining:
            chunk = batch.read(min(remaining, 1 << 20))
            if not chunk:
                raise Mismatch(f"{what}: file shorter than blob")
            if handle.read(len(chunk)) != chunk:
                raise Mismatch(f"{what}: content differs")
            remaining -= len(chunk)
        if handle.read(1):
            raise Mismatch(f"{what}: file longer than blob")


def walk_paths(root):
    """Yield every non-directory path relative to root.

    Only the root repository's `.git` metadata is skipped. A nested `.git`
    directory or gitdir file is unverified content and is rejected.
    Symlinked directories are reported as paths and not descended.
    """
    root = os.path.abspath(root)
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        if dirpath == root:
            if ".git" in dirnames:
                dirnames.remove(".git")
            if ".git" in filenames:
                filenames.remove(".git")
        elif ".git" in dirnames or ".git" in filenames:
            rel = os.path.relpath(dirpath, root).replace(os.sep, "/")
            raise Mismatch(f"nested .git metadata under {rel}")
        for name in list(dirnames):
            full = os.path.join(dirpath, name)
            if os.path.islink(full):
                dirnames.remove(name)
                filenames.append(name)
        for name in filenames:
            full = os.path.join(dirpath, name)
            yield os.path.relpath(full, root).replace(os.sep, "/")


def check_symlinks(root):
    root = os.path.abspath(root)
    real_root = os.path.realpath(root)
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        if dirpath == root:
            if ".git" in dirnames:
                dirnames.remove(".git")
            if ".git" in filenames:
                filenames.remove(".git")
        for entry in dirnames + filenames:
            path = os.path.join(dirpath, entry)
            if not os.path.islink(path):
                continue
            target = os.readlink(path)
            if os.path.isabs(target):
                raise Mismatch(f"absolute symlink: {path} -> {target}")
            resolved = os.path.realpath(os.path.join(dirpath, target))
            if resolved != real_root and not resolved.startswith(real_root + os.sep):
                raise Mismatch(f"escaping symlink: {path} -> {target}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory")
    parser.add_argument("revision")
    parser.add_argument("--allow-extra", action="append", default=[])
    parser.add_argument("--home", default="/nonexistent")
    args = parser.parse_args()
    root = os.path.abspath(args.directory)
    env = git_env(args.home)

    try:
        raw = run_git(root, env, "ls-tree", "-r", "-z", args.revision).stdout
        entries = parse_tree(raw)

        batch = Batch(root, env)
        try:
            for mode, sha, path in entries:
                full = os.path.join(root, path)
                size = batch.header(sha)
                if mode == "120000":
                    target = batch.read(size)
                    batch.skip_newline()
                    if not os.path.islink(full):
                        raise Mismatch(f"symlink expected: {path}")
                    if os.fsencode(os.readlink(full)) != target:
                        raise Mismatch(f"symlink target differs: {path}")
                else:
                    stream_matches(full, size, batch, path)
                    batch.skip_newline()
                    if mode not in ("100644", "100755"):
                        raise Mismatch(f"unsupported file mode {mode} for {path!r}")
                    executable = bool(os.stat(full).st_mode & 0o100)
                    if executable != (mode == "100755"):
                        raise Mismatch(f"executable bit differs: {path}")
        finally:
            batch.close()

        tracked = {path for _, _, path in entries}
        for rel in walk_paths(root):
            if rel in tracked:
                continue
            if any(rel == p or rel.startswith(p.rstrip("/") + "/") for p in args.allow_extra):
                continue
            raise Mismatch(f"unexpected extra file: {rel}")

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
