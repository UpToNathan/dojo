#!/usr/bin/env python3
"""Generate src/launcher_core/emoji-data.txt, the emoji table the launcher embeds.

Sources, both from the repository's nixpkgs pin (devenv.yaml), read at generation time only:
  unicode-emoji     17.0.0  share/unicode/emoji/emoji-test.txt   code points, names, groups, version
  cldr-annotations  48.2    share/unicode/cldr/common/annotations{,Derived}/en.xml   keywords

Nothing here runs on the desktop: the launcher embeds the checked-in output and never reads Unicode
or CLDR files, downloads anything or starts devenv.

Usage:
  scripts/generate-emoji-data.py            regenerate the data file (runs `nix build` for the sources)
  scripts/generate-emoji-data.py --check    fail if the checked-in file differs from a fresh run
  scripts/generate-emoji-data.py --emoji-dir DIR --cldr-dir DIR --out FILE
        use already-built package outputs (DIR is the store path of the package)

Output format, one record per line, tab separated, in CLDR order (the launcher's default order):
  @group <name>                 starts a group
  @subgroup <name>              starts a subgroup
  glyph NAME VERSION KEYWORDS TONES
        KEYWORDS  CLDR short names and the text-to-speech name, joined with "|"
        TONES     the five skin-tone variants of the glyph in Unicode order (light to dark), joined
                  with a space, or empty

What is kept: fully-qualified emoji outside the Component group. A skin-tone variant whose tone
modifiers all name one tone folds under the emoji with the modifiers removed; sequences that mix
tones (a handshake with two different tones) are dropped.

The Unicode data files are under the Unicode License v3; see docs/licenses/unicode-data.txt.
"""
import argparse
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src/launcher_core/emoji-data.txt"
TONES = [0x1F3FB, 0x1F3FC, 0x1F3FD, 0x1F3FE, 0x1F3FF]
VS16 = "️"
LINE = re.compile(r"^([0-9A-F ]+?)\s*;\s*(\S+)\s*# \S+ E(\d+\.\d+) (.+)$")


def nixpkgs_url():
    text = (ROOT / "devenv.yaml").read_text()
    match = re.search(r"^  nixpkgs:\n    url: (\S+)", text, re.M)
    if not match:
        sys.exit("devenv.yaml has no nixpkgs input")
    return match.group(1)


def build(attr):
    out = subprocess.run(
        ["nix", "build", "--no-link", "--print-out-paths", f"{nixpkgs_url()}#{attr}"],
        check=True, capture_output=True, text=True,
    ).stdout.split()
    return Path(out[0])


def package_label(path):
    """`unicode-emoji 17.0.0` from /nix/store/<hash>-unicode-emoji-17.0.0."""
    name = path.name.split("-", 1)[1]
    match = re.match(r"(.+)-(\d[\w.]*)$", name)
    return f"{match.group(1)} {match.group(2)}" if match else name


def parse_test_file(path):
    """Fully-qualified emoji as (glyph, name, version, group, subgroup), in file order."""
    group = subgroup = None
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("# group: "):
            group = line[len("# group: "):]
        elif line.startswith("# subgroup: "):
            subgroup = line[len("# subgroup: "):]
        elif line and not line.startswith("#"):
            match = LINE.match(line)
            if not match:
                sys.exit(f"unrecognised emoji-test line: {line!r}")
            codes, status, version, name = match.groups()
            if status != "fully-qualified" or group == "Component":
                continue
            glyph = "".join(chr(int(code, 16)) for code in codes.split())
            rows.append((glyph, name, version, group, subgroup))
    return rows


def parse_annotations(directory):
    """Map glyph (VS16 removed, as CLDR writes it) to (keywords, tts), merging both CLDR folders."""
    table = {}
    for folder in ("annotations", "annotationsDerived"):
        root = ET.parse(directory / folder / "en.xml").getroot()
        for node in root.iter("annotation"):
            glyph = node.get("cp")
            keywords, tts = table.setdefault(glyph, ([], []))
            text = (node.text or "").strip()
            if node.get("type") == "tts":
                tts.append(text)
            else:
                keywords.extend(word.strip() for word in text.split("|") if word.strip())
    return table


def tone_of(glyph):
    """The single tone every modifier in the glyph names, or None for none or mixed tones."""
    found = {ord(char) for char in glyph if ord(char) in TONES}
    return found.pop() if len(found) == 1 else None


def has_tone(glyph):
    return any(ord(char) in TONES for char in glyph)


def strip_tones(glyph):
    return "".join(char for char in glyph if ord(char) not in TONES)


def generate(emoji_dir, cldr_dir):
    rows = parse_test_file(emoji_dir / "share/unicode/emoji/emoji-test.txt")
    annotations = parse_annotations(cldr_dir / "share/unicode/cldr/common")

    bases = [row for row in rows if not has_tone(row[0])]
    # Tone variants drop the emoji presentation selector their base carries (a weight lifter is
    # U+1F3CB U+FE0F, its tone variants U+1F3CB U+1F3FB), so match bases without it.
    base_by_key = {row[0].replace(VS16, ""): row[0] for row in bases}
    tones = {row[0]: {} for row in bases}
    dropped_mixed = unmatched = 0
    for glyph, *_ in rows:
        if not has_tone(glyph):
            continue
        tone = tone_of(glyph)
        base = base_by_key.get(strip_tones(glyph).replace(VS16, ""))
        if tone is None:
            dropped_mixed += 1
        elif base is None:
            unmatched += 1
        else:
            tones[base][tone] = glyph

    lines = []
    group = subgroup = None
    kept = missing_keywords = 0
    for glyph, name, version, row_group, row_subgroup in bases:
        if row_group != group:
            group, subgroup = row_group, None
            lines.append(f"@group {group}")
        if row_subgroup != subgroup:
            subgroup = row_subgroup
            lines.append(f"@subgroup {subgroup}")
        keywords, tts = annotations.get(glyph.replace(VS16, ""), ([], []))
        words = []
        for word in [*keywords, *tts]:
            word = word.lower()
            if word != name.lower() and word not in words:
                words.append(word)
        missing_keywords += not words
        variants = tones[glyph]
        # Either all five tones are present or the emoji has none; a partial set would misalign the
        # tone picker, so refuse rather than guess.
        if variants and sorted(variants) != TONES:
            sys.exit(f"{name}: incomplete tone set {sorted(variants)}")
        tone_field = " ".join(variants[tone] for tone in TONES) if variants else ""
        for field in (glyph, name, "|".join(words), tone_field):
            if "\t" in field or "\n" in field:
                sys.exit(f"{name}: tab or newline in a field")
        lines.append("\t".join([glyph, name, version, "|".join(words), tone_field]))
        kept += 1

    header = [
        "# Generated by scripts/generate-emoji-data.py. Do not edit; regenerate it.",
        f"# Sources: {package_label(emoji_dir)} (emoji-test.txt), {package_label(cldr_dir)} (en annotations).",
        "# Unicode License v3, see docs/licenses/unicode-data.txt.",
        "# Format: see the generator header. Tab separated: glyph, name, Unicode emoji version, keywords, tones.",
    ]
    print(
        f"{kept} emoji, {sum(1 for t in tones.values() if t)} with tones, "
        f"{dropped_mixed} mixed-tone sequences dropped, {unmatched} tone variants without a base, "
        f"{missing_keywords} with a name and no other keywords",
        file=sys.stderr,
    )
    if unmatched:
        sys.exit("tone variants without a base emoji; extend the folding rule")
    return "\n".join([*header, *lines]) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--emoji-dir", type=Path, help="store path of unicode-emoji")
    parser.add_argument("--cldr-dir", type=Path, help="store path of cldr-annotations")
    parser.add_argument("--out", type=Path, default=OUT)
    parser.add_argument("--check", action="store_true", help="compare with the checked-in file instead of writing")
    args = parser.parse_args()
    emoji_dir = args.emoji_dir or build("unicode-emoji")
    cldr_dir = args.cldr_dir or build("cldr-annotations")
    text = generate(emoji_dir, cldr_dir)
    if args.check:
        if args.out.read_text(encoding="utf-8") != text:
            sys.exit(f"{args.out} is out of date; run scripts/generate-emoji-data.py")
        print(f"{args.out} is up to date", file=sys.stderr)
    else:
        args.out.write_text(text, encoding="utf-8")
        print(f"wrote {args.out} ({len(text.encode())} bytes)", file=sys.stderr)


if __name__ == "__main__":
    main()
