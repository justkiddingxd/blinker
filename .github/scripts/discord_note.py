"""The note blinker's bot posts in the Discord channel after a build
(release.yml): a release that came out, or a build that passed or broke.

Prints the message as JSON. Reads the run from the environment: GITHUB_*,
WIN / MAC / REL (the jobs' results), RUN_URL. `--preview` prints every kind
of note instead (a list), made up, to look at them.
"""

import json
import os
import subprocess
import sys
from datetime import datetime, timezone

SITE = "https://blinker.cool"
LOGO = f"{SITE}/assets/discord-logo.png"
COLORS = {"release": 0xD8D5CD, "passed": 0x3BA55D, "failed": 0xE5484D, "cancelled": 0x80848E}
STATE = {
    "success": "✅ built",
    "failure": "❌ failed",
    "cancelled": "⏹️ cancelled",
    "skipped": "➖ skipped",
}


def english_notes(version):
    """The English half of the version's CHANGELOG.md section: (heading, [items])."""
    section, on = [], False
    with open("CHANGELOG.md", encoding="utf-8") as f:
        for line in f:
            if line.rstrip() == f"## {version}":
                on = True
                continue
            if on and line.startswith("## "):
                break
            if on:
                section.append(line.rstrip("\n"))
    if "---" in [l.strip() for l in section]:
        section = section[[l.strip() for l in section].index("---") + 1 :]
    groups, heading = [], "What's new"
    for line in section:
        line = line.strip()
        if line.startswith("**") and line.endswith("**"):
            heading = line.strip("*")
        elif line.startswith("- "):
            if not groups or groups[-1][0] != heading:
                groups.append((heading, []))
            groups[-1][1].append(line[2:])
    return groups


def button(label, url):
    return {"type": 2, "style": 5, "label": label, "url": url}


def release_note(repo, tag, groups):
    v = tag.lstrip("v")
    page = f"https://github.com/{repo}/releases/tag/{tag}"
    files = f"https://github.com/{repo}/releases/download/{tag}"
    return {
        "embeds": [
            {
                "title": f"blinker {v} is out",
                "url": page,
                # what changed, by part; the details are on GitHub
                "description": "\n".join(f"• {heading}" for heading, _ in groups) + f"\n\n[What's new in {v}]({page})",
                "color": COLORS["release"],
                "thumbnail": {"url": LOGO},
                "footer": {"text": f"{tag} · Windows & macOS"},
                "timestamp": datetime.now(timezone.utc).isoformat(),
            }
        ],
        "components": [
            {
                "type": 1,
                "components": [
                    # blinker.cool/update hands it to the app (blinker://update):
                    # Discord links only to web pages
                    button("Update", f"{SITE}/update"),
                    button("Windows", f"{files}/blinker.exe"),
                    button("macOS", f"{files}/Blinker.dmg"),
                ],
            }
        ],
    }


def build_note(kind, what, commit, results, run_url):
    title = {"passed": f"{what} passed", "failed": f"{what} failed", "cancelled": f"{what} cancelled"}[kind]
    fields = [{"name": name, "value": STATE.get(r, r), "inline": True} for name, r in results]
    # no buttons: the title links to the run
    return {
        "embeds": [
            {
                "author": {"name": "blinker · builds", "icon_url": LOGO},
                "title": title,
                "url": run_url,
                "description": commit,
                "color": COLORS[kind],
                "fields": fields,
                "timestamp": datetime.now(timezone.utc).isoformat(),
            }
        ],
    }


def commit_line():
    sha = os.environ.get("GITHUB_SHA", "")[:7]
    try:
        msg = subprocess.run(["git", "log", "-1", "--format=%s"], capture_output=True, text=True).stdout.strip()
    except OSError:
        msg = ""
    return f"`{sha}` {msg}".strip()


def note():
    env = os.environ
    repo, ref = env["GITHUB_REPOSITORY"], env["GITHUB_REF_NAME"]
    win, mac, rel = env.get("WIN", ""), env.get("MAC", ""), env.get("REL", "skipped")
    tagged = env.get("GITHUB_REF_TYPE") == "tag"
    if tagged and rel == "success":
        return release_note(repo, ref, english_notes(ref.lstrip("v")))
    what = f"Release {ref}" if tagged else f"Build · {ref}"
    results = [("Windows", win), ("macOS", mac)] + ([("Release", rel)] if tagged else [])
    states = {win, mac, rel if tagged else "success"}
    kind = "passed" if states == {"success"} else "cancelled" if "cancelled" in states and "failure" not in states else "failed"
    return build_note(kind, what, commit_line(), results, env["RUN_URL"])


def preview():
    repo, run = "justkiddingxd/blinker", "https://github.com/justkiddingxd/blinker/actions/runs/1234567890"
    commit = "`fa72ec3` blinker 0.4.1: clip editor cuts, subtitles from speech"
    return [
        ("release is out", release_note(repo, "v0.4.1", english_notes("0.4.1"))),
        ("manual build passed", build_note("passed", "Build · main", commit, [("Windows", "success"), ("macOS", "success")], run)),
        ("release failed", build_note("failed", "Release v0.4.1", commit, [("Windows", "success"), ("macOS", "failure"), ("Release", "skipped")], run)),
        ("manual build failed", build_note("failed", "Build · main", commit, [("Windows", "failure"), ("macOS", "success")], run)),
        ("release cancelled", build_note("cancelled", "Release v0.4.1", commit, [("Windows", "cancelled"), ("macOS", "cancelled"), ("Release", "skipped")], run)),
    ]


if __name__ == "__main__":
    print(json.dumps(preview() if "--preview" in sys.argv else note(), indent=1))
