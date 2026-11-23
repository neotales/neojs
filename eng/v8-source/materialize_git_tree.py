#!/usr/bin/env python3
"""Materialise a git commit's tree into an empty directory, without checkout.

Usage:
    materialize_git_tree.py GIT_DIR REV DEST [--allow-existing-git] [--home HOME]

Content comes from the object database via `git cat-file`, not from the working
tree, the index, `.gitattributes` filters or ignored files. Regular files are
written 0644 or 0755; symlinks are recreated only when they resolve inside DEST;
gitlinks and unknown modes are refused. DEST must be absent or empty.
"""

import os
import posixpath
import stat
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


class Invalid(Exception):
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


def safe_path(name):
    if not name or "\x00" in name:
        raise Invalid(f"unsafe path: {name!r}")
    if name.startswith("/"):
        raise Invalid(f"absolute path in tree: {name!r}")
    for part in name.split("/"):
        if part in ("", ".", ".."):
            raise Invalid(f"unsafe component in tree path: {name!r}")
    return name


def parse_tree(raw):
    entries = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        try:
            meta, path = item.split(b"\t", 1)
            mode, kind, sha = meta.split(b" ")
        except ValueError as exc:
            raise Invalid(f"malformed tree entry {item!r}") from exc
        if mode == b"160000":
            raise Invalid(f"gitlink/submodule not allowed: {path!r}")
        if kind != b"blob":
            raise Invalid(f"unsupported tree entry {kind!r} for {path!r}")
        entries.append((mode.decode(), sha.decode(), safe_path(os.fsdecode(path))))
    return entries


class Batch:
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
            raise Invalid(f"cannot read blob {sha}")
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


def ensure_parent(root, path):
    parent = os.path.dirname(path)
    os.makedirs(parent, mode=0o755, exist_ok=True)
    real_root = os.path.realpath(root)
    if not os.path.isdir(parent) or os.path.islink(parent):
        raise Invalid(f"parent is not a real directory: {parent!r}")
    resolved = os.path.realpath(parent)
    if resolved != real_root and not resolved.startswith(real_root + os.sep):
        raise Invalid(f"parent escapes destination: {parent!r}")


def materialize(git_dir, rev, dest, home, allow_existing_git):
    env = git_env(home)
    proc = subprocess.run(
        GIT_ARGS + ["ls-tree", "-r", "-z", rev], cwd=git_dir, env=env,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.decode(errors="replace").strip())
    entries = parse_tree(proc.stdout)

    if os.path.lexists(dest):
        if not os.path.isdir(dest) or os.path.islink(dest):
            raise Invalid(f"destination is not a directory: {dest}")
        existing = set(os.listdir(dest))
        if existing and not (allow_existing_git and existing == {".git"}):
            raise Invalid(f"destination is not empty: {dest}")
    else:
        os.makedirs(dest, mode=0o755)

    batch = Batch(git_dir, env)
    try:
        for mode, sha, name in entries:
            path = os.path.join(dest, *name.split("/"))
            ensure_parent(dest, path)
            if os.path.lexists(path):
                raise Invalid(f"duplicate destination path: {name!r}")
            size = batch.header(sha)
            if mode == "120000":
                target = os.fsdecode(batch.read(size))
                batch.skip_newline()
                if not target or "\x00" in target or os.path.isabs(target):
                    raise Invalid(f"unsafe symlink target: {name!r} -> {target!r}")
                resolved = posixpath.normpath(
                    posixpath.join(posixpath.dirname(name), target)
                )
                if resolved == ".." or resolved.startswith("../") or resolved.startswith("/"):
                    raise Invalid(f"escaping symlink: {name!r} -> {target!r}")
                os.symlink(target, path)
                continue
            if mode not in ("100644", "100755"):
                raise Invalid(f"unsupported file mode {mode} for {name!r}")
            file_mode = 0o755 if mode == "100755" else 0o644
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            try:
                with os.fdopen(fd, "wb") as out:
                    remaining = size
                    while remaining:
                        chunk = batch.read(min(remaining, 1 << 20))
                        if not chunk:
                            raise Invalid(f"short blob for {name!r}")
                        out.write(chunk)
                        remaining -= len(chunk)
            finally:
                os.chmod(path, file_mode)
            batch.skip_newline()
    finally:
        batch.close()


def main():
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("git_dir")
    parser.add_argument("rev")
    parser.add_argument("dest")
    parser.add_argument("--allow-existing-git", action="store_true")
    parser.add_argument("--home", default="/nonexistent")
    args = parser.parse_args()
    try:
        materialize(
            args.git_dir, args.rev, args.dest, args.home, args.allow_existing_git
        )
    except Invalid as exc:
        print(f"materialize_git_tree: {exc}", file=sys.stderr)
        return 2
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"materialize_git_tree: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
