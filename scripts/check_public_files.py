#!/usr/bin/env python3
"""Screen tracked files without printing matched values. Not an exhaustive secret audit."""
import re
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
names = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
patterns = [
    ("GitHub credential", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{30,}\b")),
    ("GitHub fine-grained credential", re.compile(r"\bgithub_pat_[A-Za-z0-9_]{40,}\b")),
    ("Telegram credential", re.compile(r"\b\d{6,}:[A-Za-z0-9_-]{30,}\b")),
    ("private signing material", re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")),
    ("URL credential", re.compile(r"https?://[^\s/]+:[^\s/@]+@")),
    ("URL query credential", re.compile(r"[?&](?:api[_-]?key|token|secret|password)=[A-Za-z0-9_-]{12,}", re.I)),
    ("configured endpoint assignment", re.compile(r"^STARKNET_RPC(?:_WS)?_URL[ \t]*=[ \t]*\S+", re.M)),
    ("configured bot assignment", re.compile(r"^TELEGRAM_(?:BOT_TOKEN|CHAT_ID)[ \t]*=[ \t]*\S+", re.M)),
]
problems = []
for name in filter(None, names):
    path = root / name
    if path.is_symlink():
        problems.append((name, "tracked symlink requires manual review"))
        continue
    if path.name.startswith(".env") and path.name != ".env.example":
        problems.append((name, "secret configuration file"))
        continue
    if name.startswith(("data/", "target/")) or path.suffix in {".sqlite", ".db", ".log", ".key", ".pem"}:
        problems.append((name, "runtime or sensitive artifact"))
        continue
    text = path.read_text(encoding="utf-8")
    for label, pattern in patterns:
        if pattern.search(text):
            problems.append((name, label))

if problems:
    for name, label in problems:
        print(f"BLOCKED: {name}: {label}; matched content suppressed", file=sys.stderr)
    sys.exit(1)
print(f"Public-file screening passed for {sum(bool(n) for n in names)} tracked files.")

